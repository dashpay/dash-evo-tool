//! Masternode / evonode MCP tools: identity load and credit withdrawal.
//!
//! Two tools live here, extracted from `identity.rs` (H2 refactor):
//!
//! * [`MasternodeIdentityLoad`] (`masternode_identity_load`) — bind an
//!   evonode/masternode identity by ProTxHash and private keys.
//! * [`MasternodeCreditsWithdraw`] (`masternode_credits_withdraw`) — withdraw
//!   Platform credits to a Core address using owner- or transfer-mode signing.

use std::borrow::Cow;

use dash_sdk::dpp::identity::Purpose;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use rmcp::handler::server::router::tool::{AsyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::backend_task::identity::{IdentityInputToLoad, IdentityTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::mcp::dispatch::dispatch_task;
use crate::mcp::error::McpToolError;
use crate::mcp::resolve;
use crate::mcp::server::DashMcpService;
use crate::model::masternode_input::{self, KeyMode};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::secret::Secret;

// ---------------------------------------------------------------------------
// MasternodeIdentityLoad — load a masternode/evonode identity by ProTxHash
// ---------------------------------------------------------------------------

/// Load a masternode or evonode identity by ProTxHash and private keys.
pub struct MasternodeIdentityLoad;

/// Parameters for [`MasternodeIdentityLoad`].
///
/// Private keys are typed as [`Secret`] so they are mlock-ed and
/// zeroize-on-drop from the moment they are deserialized — no intermediate
/// plain-`String` window exists before or during the SPV wait.
#[derive(Deserialize, schemars::JsonSchema, Default)]
pub struct MasternodeIdentityLoadParams {
    /// ProTxHash of the masternode/evonode (its identity ID). Accepts hex (the
    /// canonical encoding) or Base58.
    pub pro_tx_hash: String,
    /// Node type: "masternode" or "evonode".
    pub node_type: String,
    /// Owner private key (WIF or 64-char hex). Bound as the OWNER key. At
    /// least one of the owner or payout private key is required.
    #[serde(default)]
    pub owner_private_key: Secret,
    /// Voting private key (WIF or 64-char hex). Optional; binds the voter
    /// identity. Does not enable a withdrawal on its own.
    #[serde(default)]
    pub voting_private_key: Secret,
    /// Payout/transfer private key (WIF or 64-char hex). Bound as the
    /// TRANSFER key. At least one of the owner or payout private key is
    /// required.
    #[serde(default)]
    pub payout_private_key: Secret,
    /// Optional human-readable name; trimmed, empty falls back to a DPNS name.
    #[serde(default)]
    pub alias: String,
    /// Expected network (required so keys and addresses bind to the right
    /// chain).
    pub network: String,
}

// Hand-written so the three private keys never reach a log sink or the MCP
// error `data` payload — Secret's own Debug already redacts, but the struct
// debug must also render non-secret fields readably.
impl std::fmt::Debug for MasternodeIdentityLoadParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasternodeIdentityLoadParams")
            .field("pro_tx_hash", &self.pro_tx_hash)
            .field("node_type", &self.node_type)
            .field("owner_private_key", &self.owner_private_key)
            .field("voting_private_key", &self.voting_private_key)
            .field("payout_private_key", &self.payout_private_key)
            .field("alias", &self.alias)
            .field("network", &self.network)
            .finish()
    }
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct MasternodeIdentityLoadOutput {
    identity_id: String,
    node_type: String,
    alias: Option<String>,
    owner_key_loaded: bool,
    voting_key_loaded: bool,
    payout_key_loaded: bool,
    /// Withdrawal key modes this identity supports ("owner" / "transfer").
    available_withdrawal_keys: Vec<String>,
    /// Registered payout address (the OWNER-mode withdrawal destination), if
    /// any.
    payout_address: Option<String>,
    dpns_names: Vec<String>,
}

impl ToolBase for MasternodeIdentityLoad {
    type Parameter = MasternodeIdentityLoadParams;
    type Output = MasternodeIdentityLoadOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "masternode_identity_load".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Load a masternode or evonode identity by ProTxHash, binding its \
             owner/voting/payout private keys (WIF or hex) and persisting it \
             locally for the withdraw tool. The output reports which keys \
             loaded, the available withdrawal modes, and the registered payout \
             address. The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(false)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for MasternodeIdentityLoad {
    async fn invoke(
        service: &DashMcpService,
        param: MasternodeIdentityLoadParams,
    ) -> Result<MasternodeIdentityLoadOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;

        // ── Cheap pre-flight validation (before the SPV wait) ───────────────
        // Network presence/match, node type, key presence, and ProTxHash
        // format are all pure checks that reject without touching the network.
        resolve::require_network(&ctx, Some(&param.network))?;
        let identity_type = masternode_input::parse_node_type(&param.node_type)?;
        // Keys are `Secret`-typed; presence check uses `is_blank()` so no
        // expose_secret() call is needed here.
        masternode_input::require_at_least_one_signing_key(
            &param.owner_private_key,
            &param.payout_private_key,
        )?;
        // S2: validate the ProTxHash format BEFORE the SPV wait so a malformed
        // hash is rejected immediately rather than after a 10-min sync.
        masternode_input::decode_identity_id(&param.pro_tx_hash)?;

        // ── SPV gate ─────────────────────────────────────────────────────────
        // Identity load fetches over DAPI; proof verification needs a synced
        // chain.
        resolve::ensure_spv_synced(&ctx).await?;

        let input = IdentityInputToLoad {
            identity_id_input: param.pro_tx_hash,
            identity_type,
            alias_input: param.alias.trim().to_owned(),
            // Keys are moved out of the Secret wrapper directly into the
            // backend input — no intermediate plain-String copy.
            voting_private_key_input: param.voting_private_key,
            owner_private_key_input: param.owner_private_key,
            payout_address_private_key_input: param.payout_private_key,
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            // FR-8 load-time key encryption is GUI-only this iteration
            // (requirements §2.3): this tool is a confirmed keyless (Tier-1)
            // entry point, so it loads unprotected.
            // TODO: headless password parity — accept an optional encryption
            // password param and thread it here once MCP secret handling is
            // designed.
            encryption_password: None,
            // Headless load uses overwrite/upsert semantics: a repeat load
            // REPLACES the stored node's keys with those supplied in this call.
            // Keys omitted this time are dropped — this is a full replace, not a
            // merge, so a partial-key repeat load is destructive. Duplicate
            // rejection and key-preserving merge are GUI-form affordances, not a
            // headless contract.
            // TODO: headless merge parity — expose a load-mode param so callers
            // can request MergeIntoExisting (key-preserving) once MCP secret
            // handling for Tier-2 nodes is designed.
            load_mode: crate::backend_task::identity::IdentityLoadMode::Overwrite,
            // A headless load gates on nothing: it opens a record of its own rather
            // than adopting one a GUI screen is waiting on.
            load_token: None,
        };

        let task = BackendTask::IdentityTask(IdentityTask::LoadIdentity(input));
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        let BackendTaskSuccessResult::LoadedIdentity(qi) = result else {
            return Err(McpToolError::Internal(format!(
                "Unexpected task result: {result:?}"
            )));
        };

        let mut owner_key_loaded = false;
        let mut payout_key_loaded = false;
        let mut available_withdrawal_keys = Vec::new();
        for key in qi.available_withdrawal_keys() {
            match key.identity_public_key.purpose() {
                Purpose::OWNER => {
                    // Guard: push the mode string only on first OWNER key seen so
                    // identities with multiple OWNER keys don't yield duplicates.
                    if !owner_key_loaded {
                        available_withdrawal_keys.push(KeyMode::Owner.to_string());
                    }
                    owner_key_loaded = true;
                }
                Purpose::TRANSFER => {
                    if !payout_key_loaded {
                        available_withdrawal_keys.push(KeyMode::Transfer.to_string());
                    }
                    payout_key_loaded = true;
                }
                _ => {}
            }
        }

        Ok(MasternodeIdentityLoadOutput {
            identity_id: qi.identity.id().to_string(Encoding::Base58),
            node_type: identity_type.to_string().to_ascii_lowercase(),
            alias: qi.alias.clone(),
            owner_key_loaded,
            voting_key_loaded: qi.associated_voter_identity.is_some(),
            payout_key_loaded,
            available_withdrawal_keys,
            payout_address: qi
                .masternode_payout_address(ctx.network())
                .map(|addr| addr.to_string()),
            dpns_names: qi.dpns_names.iter().map(|d| d.name.clone()).collect(),
        })
    }
}

// ---------------------------------------------------------------------------
// MasternodeCreditsWithdraw — masternode-aware Platform credits → Core
// ---------------------------------------------------------------------------

/// Withdraw a masternode or evonode identity's Platform credits to Core,
/// honouring the two key modes (owner → payout-forced, transfer → any Core
/// address).
pub struct MasternodeCreditsWithdraw;

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
pub struct MasternodeCreditsWithdrawParams {
    /// Base58 identity ID of the loaded masternode/evonode identity.
    pub identity_id: String,
    /// Key mode: "owner" (destination forced to the payout address) or
    /// "transfer" (withdraw to any Core address).
    pub key_mode: String,
    /// Core address to receive the withdrawal. Required for "transfer" mode;
    /// forbidden for "owner" mode (the destination is the registered payout
    /// address).
    #[serde(default)]
    pub to_address: String,
    /// Amount in credits to withdraw (must be greater than zero).
    pub amount_credits: u64,
    /// Expected network (required for destructive operations).
    pub network: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct MasternodeCreditsWithdrawOutput {
    pub identity_id: String,
    /// The key mode used: "owner" or "transfer" (canonical via [`KeyMode`]).
    pub key_mode: String,
    /// The Core address the funds were actually sent to (the payout address in
    /// owner mode, the caller's address in transfer mode).
    pub to_address: String,
    pub amount_credits: u64,
    pub estimated_fee: u64,
    pub actual_fee: u64,
}

/// Reject a caller-supplied address in owner mode.
///
/// An owner-key withdrawal always goes to the registered payout address, so a
/// supplied `to_address` is a contradiction surfaced as an error, not ignored.
fn reject_owner_address_contradiction(to_address: &str) -> Result<(), McpToolError> {
    if !to_address.trim().is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "An owner-key withdrawal always goes to the registered payout address. \
                      Remove 'to_address', or use key_mode=transfer to choose an address."
                .to_owned(),
        });
    }
    Ok(())
}

/// Parse a transfer-mode destination as a Core address (format only).
///
/// Rejects empties, Platform bech32m addresses (via `is_platform_address_string`,
/// not the weaker first-char check), and non-Core strings. The caller enforces
/// the network match on the returned `NetworkUnchecked` address.
fn parse_transfer_core_address(
    to_address: &str,
) -> Result<
    dash_sdk::dashcore_rpc::dashcore::Address<
        dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
    >,
    McpToolError,
> {
    let trimmed = to_address.trim();
    if trimmed.is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "A transfer withdrawal needs a Core address. \
                      Provide 'to_address' with a Core address."
                .to_owned(),
        });
    }
    if crate::model::address::is_platform_address_string(trimmed) {
        return Err(McpToolError::InvalidParam {
            message: "Enter a valid Core address — Platform addresses cannot receive withdrawals."
                .to_owned(),
        });
    }
    trimmed
        .parse::<dash_sdk::dashcore_rpc::dashcore::Address<
            dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked,
        >>()
        .map_err(|_| McpToolError::InvalidParam {
            message: "Enter a valid Core address — that address could not be read.".to_owned(),
        })
}

/// The resolved key + destination for a masternode credit withdrawal.
#[derive(Debug)]
struct WithdrawalPlan {
    /// Signing key resolved from the identity's available withdrawal keys.
    key_id: dash_sdk::dpp::identity::KeyID,
    /// Destination passed to the state transition. `None` in owner mode —
    /// Platform consensus forces the registered payout address; `Some(addr)`
    /// in transfer mode.
    dispatch_address: Option<dash_sdk::dpp::dashcore::Address>,
    /// Destination the funds actually reach, echoed to the caller: the
    /// registered payout address in owner mode, the caller's address in
    /// transfer mode.
    echo_address: dash_sdk::dpp::dashcore::Address,
}

/// Resolve the signing key and destination for a withdrawal (pure, no network).
///
/// Owner mode dispatches `None` (Platform consensus forces the registered payout
/// address — matching the GUI), validating only that a payout address exists and
/// echoing it back. Transfer mode dispatches and echoes the caller's Core
/// address after a format + active-network check.
///
/// # Errors
///
/// [`McpToolError::InvalidParam`] when the mode's key is not loaded, owner mode
/// has no registered payout address, or the transfer address is missing,
/// malformed, a Platform address, or for the wrong network.
fn resolve_withdrawal_plan(
    qi: &QualifiedIdentity,
    key_mode: KeyMode,
    to_address: &str,
    network: dash_sdk::dpp::dashcore::Network,
) -> Result<WithdrawalPlan, McpToolError> {
    let purpose = match key_mode {
        KeyMode::Owner => Purpose::OWNER,
        KeyMode::Transfer => Purpose::TRANSFER,
    };
    let key_id = qi
        .available_withdrawal_keys()
        .into_iter()
        .find(|k| k.identity_public_key.purpose() == purpose)
        .map(|k| k.identity_public_key.id())
        .ok_or_else(|| McpToolError::InvalidParam {
            message: match key_mode {
                KeyMode::Owner => "The owner key (owner_private_key) needed for owner-mode \
                                   withdrawals is not loaded. Re-run masternode-identity-load \
                                   with owner_private_key included."
                    .to_owned(),
                KeyMode::Transfer => "The payout key (payout_private_key) needed for \
                                      transfer-mode withdrawals is not loaded. Re-run \
                                      masternode-identity-load with payout_private_key included."
                    .to_owned(),
            },
        })?;

    match key_mode {
        KeyMode::Owner => {
            let payout = qi.masternode_payout_address(network).ok_or_else(|| {
                McpToolError::InvalidParam {
                    message: "This identity has no registered payout address, so an owner-key \
                              withdrawal has no destination. Use key_mode=transfer with a Core \
                              address."
                        .to_owned(),
                }
            })?;
            Ok(WithdrawalPlan {
                key_id,
                dispatch_address: None,
                echo_address: payout,
            })
        }
        KeyMode::Transfer => {
            let checked = parse_transfer_core_address(to_address)?
                .require_network(network)
                .map_err(|_| McpToolError::InvalidParam {
                    message: "The Core address does not match the active network. \
                              Use an address for the active network."
                        .to_owned(),
                })?;
            Ok(WithdrawalPlan {
                key_id,
                dispatch_address: Some(checked.clone()),
                echo_address: checked,
            })
        }
    }
}

impl ToolBase for MasternodeCreditsWithdraw {
    type Parameter = MasternodeCreditsWithdrawParams;
    type Output = MasternodeCreditsWithdrawOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        "masternode_credits_withdraw".into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Withdraw a loaded masternode/evonode identity's Platform credits to \
             Core. With key_mode=owner the destination is forced to the \
             registered payout address; with key_mode=transfer you choose any \
             Core address. The 'network' parameter is required."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(
            ToolAnnotations::default()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
}

impl AsyncTool<DashMcpService> for MasternodeCreditsWithdraw {
    async fn invoke(
        service: &DashMcpService,
        param: MasternodeCreditsWithdrawParams,
    ) -> Result<MasternodeCreditsWithdrawOutput, McpToolError> {
        let ctx = service.tool_ctx().await?;

        // Cheap validation first, before the SPV wait.
        resolve::require_network(&ctx, Some(&param.network))?;
        resolve::validate_positive_amount(param.amount_credits, "credits")?;
        let key_mode = masternode_input::parse_key_mode(&param.key_mode)?;

        // Surface the owner+address contradiction before resolving the identity
        // so it fires even for a not-yet-loaded identity.
        if key_mode == KeyMode::Owner {
            reject_owner_address_contradiction(&param.to_address)?;
        }

        // Parse identity ID (pure — no backend access).
        let identity_id = masternode_input::decode_identity_id(&param.identity_id)?;

        // Wire the wallet backend and wait for a synced chain BEFORE any
        // backend-touching call.
        resolve::ensure_spv_synced(&ctx).await?;

        // Resolve the loaded identity. A not-found points at the load tool.
        let qi = ctx
            .get_identity_by_id(&identity_id)
            .map_err(|e| McpToolError::Internal(e.to_string()))?
            .ok_or_else(|| McpToolError::InvalidParam {
                message: format!(
                    "This identity is not loaded yet: {}. \
                     Run masternode-identity-load with the ProTxHash and keys first.",
                    param.identity_id
                ),
            })?;

        // Resolve the signing key and destination per mode (pure, no network).
        let plan = resolve_withdrawal_plan(&qi, key_mode, &param.to_address, ctx.network())?;

        let task = BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
            qi,
            plan.dispatch_address,
            param.amount_credits,
            Some(plan.key_id),
        ));
        let result = dispatch_task(&ctx, task)
            .await
            .map_err(McpToolError::TaskFailed)?;

        let BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) = result else {
            return Err(McpToolError::Internal(format!(
                "Unexpected task result: {result:?}"
            )));
        };

        Ok(MasternodeCreditsWithdrawOutput {
            identity_id: param.identity_id,
            // S3: use KeyMode Display — single source of truth for "owner"/"transfer".
            key_mode: key_mode.to_string(),
            // Echo the destination the funds actually go to (the registered
            // payout address in owner mode), so the caller always learns where.
            to_address: plan.echo_address.to_string(),
            amount_credits: param.amount_credits,
            estimated_fee: fee_result.estimated_fee,
            actual_fee: fee_result.actual_fee.unwrap_or(fee_result.estimated_fee),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    // ── Tool A param Debug redaction (TC-MN-010) ──────────────────────────
    //
    // The single most important security test: the params `Debug` must never
    // surface any of the three private keys, because `McpToolError::TaskFailed`
    // serializes `{task_err:?}` into the MCP error `data` payload.

    #[test]
    fn load_params_debug_redacts_every_private_key() {
        let params = MasternodeIdentityLoadParams {
            pro_tx_hash: "PROTX_HASH_VALUE".to_owned(),
            node_type: "evonode".to_owned(),
            owner_private_key: Secret::new("OWNER_SECRET_VALUE"),
            voting_private_key: Secret::new("VOTING_SECRET_VALUE"),
            payout_private_key: Secret::new("PAYOUT_SECRET_VALUE"),
            alias: "my-node".to_owned(),
            network: "testnet".to_owned(),
        };

        let debug = format!("{params:?}");

        // No key sentinel may appear.
        assert!(
            !debug.contains("OWNER_SECRET_VALUE"),
            "owner key leaked: {debug}"
        );
        assert!(
            !debug.contains("VOTING_SECRET_VALUE"),
            "voting key leaked: {debug}"
        );
        assert!(
            !debug.contains("PAYOUT_SECRET_VALUE"),
            "payout key leaked: {debug}"
        );
        // Each key field renders as the Secret redaction marker.
        assert!(
            debug.contains("Secret(***)"),
            "keys must render as Secret(***): {debug}"
        );
        // Non-secret fields stay readable.
        assert!(
            debug.contains("PROTX_HASH_VALUE"),
            "pro_tx_hash visible: {debug}"
        );
        assert!(debug.contains("evonode"), "node_type visible: {debug}");
        assert!(debug.contains("my-node"), "alias visible: {debug}");
        assert!(debug.contains("testnet"), "network visible: {debug}");
    }

    // ── Key-format policy delegation (TC-MN-011) ──────────────────────────
    //
    // The tool feeds keys straight into Secret -> IdentityInputToLoad ->
    // the backend `verify_key_input`, which is the single source of truth for
    // the length policy.  The tool adds no competing length check.
    #[test]
    fn load_params_accept_any_key_length_without_local_check() {
        for key in ["", "tooshort", &"a".repeat(63), &"f".repeat(64)] {
            let params = MasternodeIdentityLoadParams {
                pro_tx_hash: "x".to_owned(),
                node_type: "masternode".to_owned(),
                owner_private_key: Secret::new(key),
                network: "testnet".to_owned(),
                ..Default::default()
            };
            // Construction never validates key length.
            assert_eq!(params.owner_private_key.expose_secret(), key);
        }
    }

    // ── Discoverability & schema (TC-MN-015) ──────────────────────────────

    fn schema_property_names(tool: &rmcp::model::Tool) -> Vec<String> {
        tool.input_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn load_tool_registered_with_expected_annotations_and_schema() {
        let router = DashMcpService::tool_router();
        let tool = router
            .get("masternode_identity_load")
            .expect("masternode_identity_load must be registered in tool_router");

        let ann = tool
            .annotations
            .as_ref()
            .expect("tool must carry annotations");
        assert_eq!(ann.read_only_hint, Some(false));
        assert_eq!(ann.destructive_hint, Some(false));
        assert_eq!(ann.idempotent_hint, Some(false));
        assert_eq!(ann.open_world_hint, Some(true));

        let props = schema_property_names(tool);
        for expected in [
            "pro_tx_hash",
            "node_type",
            "owner_private_key",
            "voting_private_key",
            "payout_private_key",
            "alias",
            "network",
        ] {
            assert!(
                props.iter().any(|p| p == expected),
                "schema must expose '{expected}', got {props:?}"
            );
        }
    }

    // ── Tool B pure pre-flight checks (TC-MN-031/032/033/034/035) ─────────

    #[test]
    fn withdraw_amount_zero_rejected() {
        // TC-MN-032 — delegated to the shared resolve::validate_positive_amount.
        let err = resolve::validate_positive_amount(0, "credits").unwrap_err();
        assert!(err.to_string().contains("greater than zero"), "got: {err}");
        assert!(resolve::validate_positive_amount(1, "credits").is_ok());
    }

    #[test]
    fn owner_mode_supplied_address_rejected() {
        // TC-MN-033 — a non-empty address in owner mode is a contradiction.
        let err =
            reject_owner_address_contradiction("yQ9JNCT4S9zVHaKYbr1FUY4YkUMYxSzWAj").unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(
            err.to_string().contains("registered payout address"),
            "got: {err}"
        );
        // Empty / whitespace-only address is the expected owner-mode input.
        assert!(reject_owner_address_contradiction("").is_ok());
        assert!(reject_owner_address_contradiction("   ").is_ok());
    }

    #[test]
    fn transfer_mode_missing_address_rejected() {
        // TC-MN-034 — transfer mode requires an address.
        for empty in ["", "   "] {
            let err = parse_transfer_core_address(empty).unwrap_err();
            assert!(matches!(err, McpToolError::InvalidParam { .. }));
            assert!(err.to_string().contains("Core address"), "got: {err}");
        }
    }

    #[test]
    fn transfer_mode_invalid_address_rejected() {
        // TC-MN-035 — the address is actually parsed, not first-char checked.
        let err = parse_transfer_core_address("not-an-address").unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(err.to_string().contains("could not be read"), "got: {err}");
    }

    #[test]
    fn transfer_mode_platform_address_rejected_via_guard() {
        // TC-MN-031 / TC-MN-046 — Platform bech32m addresses are rejected by
        // is_platform_address_string, NOT the weaker first-char check.
        for platform in ["dash1qwer1234", "tdash1qwer1234"] {
            let err = parse_transfer_core_address(platform).unwrap_err();
            assert!(matches!(err, McpToolError::InvalidParam { .. }));
            assert!(
                err.to_string()
                    .contains("Platform addresses cannot receive"),
                "for {platform}: {err}"
            );
        }
    }

    /// Derive a real, checksum-valid testnet P2PKH address from a fixed key.
    fn sample_core_address() -> String {
        use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dashcore_rpc::dashcore::{Address, Network, PrivateKey, PublicKey};

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[7u8; 32]).expect("valid secret key");
        let privkey = PrivateKey::new(sk, Network::Testnet);
        let pubkey = PublicKey::from_private_key(&secp, &privkey);
        Address::p2pkh(&pubkey, Network::Testnet).to_string()
    }

    #[test]
    fn transfer_mode_valid_core_address_accepted() {
        let addr = sample_core_address();
        assert!(
            parse_transfer_core_address(&addr).is_ok(),
            "derived address {addr} should parse"
        );
    }

    // ── Tool B discoverability & schema (TC-MN-043) ───────────────────────

    #[test]
    fn withdraw_tool_registered_with_expected_annotations_and_schema() {
        let router = DashMcpService::tool_router();
        let tool = router
            .get("masternode_credits_withdraw")
            .expect("masternode_credits_withdraw must be registered in tool_router");

        let ann = tool
            .annotations
            .as_ref()
            .expect("tool must carry annotations");
        assert_eq!(ann.read_only_hint, Some(false));
        assert_eq!(ann.destructive_hint, Some(true));
        assert_eq!(ann.idempotent_hint, Some(false));
        assert_eq!(ann.open_world_hint, Some(true));

        let props = schema_property_names(tool);
        for expected in [
            "identity_id",
            "key_mode",
            "to_address",
            "amount_credits",
            "network",
        ] {
            assert!(
                props.iter().any(|p| p == expected),
                "schema must expose '{expected}', got {props:?}"
            );
        }
    }

    // ── TaskFailed data payload never leaks key material (TC-MN-061) ──────

    #[test]
    fn secret_debug_never_reveals_plaintext() {
        let secret = Secret::new("OWNER_SECRET_WIF_VALUE");
        let debug = format!("{secret:?}");
        assert_eq!(debug, "Secret(***)");
        assert!(!debug.contains("OWNER_SECRET_WIF_VALUE"));
    }

    #[test]
    fn task_failed_data_payload_excludes_key_material() {
        use crate::backend_task::error::TaskError;
        use rmcp::ErrorData as McpError;

        let err = McpToolError::TaskFailed(TaskError::KeyInputValidationFailed(
            crate::model::key_input::KeyInputError::UnsupportedLength {
                key_name: "Owner".to_owned(),
            },
        ));

        let mcp: McpError = err.into();
        let message = mcp.message.to_string();
        let data = mcp.data.map(|v| v.to_string()).unwrap_or_default();

        // The typed variant name belongs in the debug `data` payload (for MCP
        // clients / developers) but must NOT appear in the user-facing `message`
        // (which uses Display, not Debug).
        const VARIANT: &str = "KeyInputValidationFailed";
        assert!(
            data.contains(VARIANT),
            "data should carry the typed variant for diagnostics: {data}"
        );
        assert!(
            !message.contains(VARIANT),
            "variant name must not leak into the user-facing message: {message}"
        );
    }

    // ── resolve_withdrawal_plan: owner→None / transfer→Some ──────────────

    use dash_sdk::dpp::identity::IdentityPublicKey;
    use dash_sdk::dpp::version::PlatformVersion;

    fn masternode_fixture(with_owner: bool, with_payout: bool) -> QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, PrivateKeyTarget};
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        use dash_sdk::platform::{Identifier, Identity};

        let pv = PlatformVersion::latest();
        let mut identity =
            Identity::create_basic_identity(Identifier::default(), pv).expect("basic identity");
        let mut key_storage = KeyStorage::default();
        let mut public_keys = BTreeMap::new();

        if with_owner {
            let (owner_key, owner_secret) =
                IdentityPublicKey::random_masternode_owner_key(0, Some(1), pv).expect("owner key");
            public_keys.insert(owner_key.id(), owner_key.clone());
            key_storage.insert_at(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, owner_key.id()),
                (
                    QualifiedIdentityPublicKey::from(owner_key),
                    PrivateKeyData::Clear(owner_secret),
                ),
            );
        }
        if with_payout {
            let (payout_key, payout_secret) =
                IdentityPublicKey::random_masternode_transfer_key(1, Some(2), pv)
                    .expect("transfer key");
            public_keys.insert(payout_key.id(), payout_key.clone());
            key_storage.insert_at(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, payout_key.id()),
                (
                    QualifiedIdentityPublicKey::from(payout_key),
                    PrivateKeyData::Clear(payout_secret),
                ),
            );
        }
        identity.set_public_keys(public_keys);

        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Evonode,
            alias: None,
            private_keys: key_storage,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    fn testnet_core_address() -> String {
        use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dashcore_rpc::dashcore::{Address, Network, PrivateKey, PublicKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[9u8; 32]).expect("valid secret key");
        let pk = PublicKey::from_private_key(&secp, &PrivateKey::new(sk, Network::Testnet));
        Address::p2pkh(&pk, Network::Testnet).to_string()
    }

    #[test]
    fn owner_mode_dispatches_none_and_echoes_payout() {
        use dash_sdk::dpp::dashcore::Network;
        let qi = masternode_fixture(true, true);
        let plan = resolve_withdrawal_plan(&qi, KeyMode::Owner, "", Network::Testnet)
            .expect("owner plan resolves");
        assert!(
            plan.dispatch_address.is_none(),
            "owner mode must dispatch None, got {:?}",
            plan.dispatch_address
        );
        let expected = qi
            .masternode_payout_address(Network::Testnet)
            .expect("fixture has a payout address");
        assert_eq!(plan.echo_address, expected);
    }

    #[test]
    fn transfer_mode_dispatches_and_echoes_the_caller_address() {
        use dash_sdk::dpp::dashcore::Network;
        let qi = masternode_fixture(false, true);
        let addr = testnet_core_address();
        let plan = resolve_withdrawal_plan(&qi, KeyMode::Transfer, &addr, Network::Testnet)
            .expect("transfer plan resolves");
        let dispatched = plan
            .dispatch_address
            .expect("transfer mode dispatches Some");
        assert_eq!(dispatched.to_string(), addr);
        assert_eq!(plan.echo_address.to_string(), addr);
    }

    #[test]
    fn owner_mode_key_not_loaded_rejected_naming_param() {
        use dash_sdk::dpp::dashcore::Network;
        let qi = masternode_fixture(false, true);
        let err = resolve_withdrawal_plan(&qi, KeyMode::Owner, "", Network::Testnet).unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(
            err.to_string().contains("owner_private_key"),
            "message names the param: {err}"
        );
    }

    #[test]
    fn transfer_mode_key_not_loaded_rejected_naming_param() {
        use dash_sdk::dpp::dashcore::Network;
        let qi = masternode_fixture(true, false);
        let addr = testnet_core_address();
        let err =
            resolve_withdrawal_plan(&qi, KeyMode::Transfer, &addr, Network::Testnet).unwrap_err();
        assert!(
            err.to_string().contains("payout_private_key"),
            "message names the param: {err}"
        );
    }

    #[test]
    fn owner_mode_no_payout_address_rejected() {
        use dash_sdk::dpp::dashcore::Network;
        let qi = masternode_fixture(true, false);
        let err = resolve_withdrawal_plan(&qi, KeyMode::Owner, "", Network::Testnet).unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(
            err.to_string().contains("no registered payout address"),
            "got: {err}"
        );
    }

    #[test]
    fn transfer_mode_cross_network_address_rejected() {
        use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
        use dash_sdk::dashcore_rpc::dashcore::{Address, Network, PrivateKey, PublicKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[3u8; 32]).expect("valid secret key");
        let pk = PublicKey::from_private_key(&secp, &PrivateKey::new(sk, Network::Mainnet));
        let mainnet_addr = Address::p2pkh(&pk, Network::Mainnet).to_string();

        let qi = masternode_fixture(false, true);
        let err = resolve_withdrawal_plan(&qi, KeyMode::Transfer, &mainnet_addr, Network::Testnet)
            .unwrap_err();
        assert!(matches!(err, McpToolError::InvalidParam { .. }));
        assert!(
            err.to_string()
                .contains("does not match the active network"),
            "got: {err}"
        );
    }

    // ── S3: KeyMode Display is the single source of truth for strings ─────

    #[test]
    fn key_mode_display_matches_parse_roundtrip() {
        use crate::model::masternode_input::parse_key_mode;
        // The Display impl and parse_key_mode must agree on the canonical strings.
        assert_eq!(KeyMode::Owner.to_string(), "owner");
        assert_eq!(KeyMode::Transfer.to_string(), "transfer");
        assert_eq!(parse_key_mode("owner").unwrap(), KeyMode::Owner);
        assert_eq!(parse_key_mode("transfer").unwrap(), KeyMode::Transfer);
    }
}
