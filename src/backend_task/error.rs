//! Typed error envelope for backend tasks.
//!
//! `Display` → user-friendly text (shown in `MessageBanner`).
//! `Debug` → variant name + fields (logged and shown in collapsible details).
//! `From<String>` → backwards compatible with existing `Result<T, String>` code.
//!   Parses known error patterns into typed variants automatically.

use dash_sdk::Error as SdkError;
use dash_sdk::dashcore_rpc;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::state::state_error::StateError;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use thiserror::Error;

/// Dash Core RPC error code: wallet file not specified (multi-wallet node).
const RPC_WALLET_NOT_SPECIFIED: i32 = -19;

/// App-level error envelope for backend tasks.
#[derive(Debug, Error)]
pub enum TaskError {
    /// Legacy string error — backwards compatible with all existing code.
    #[error("{0}")]
    Generic(String),

    /// Boxed error — catch-all for errors without a dedicated variant.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),

    /// SPV subsystem errors.
    #[error(transparent)]
    Spv(#[from] crate::spv::SpvError),

    /// DashPay domain errors.
    #[error(transparent)]
    DashPay(#[from] crate::backend_task::dashpay::errors::DashPayError),

    /// Configuration errors.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    /// GroveSTARK prover errors.
    #[error(transparent)]
    GroveStark(#[from] crate::model::grovestark_prover::GroveSTARKError),

    /// Wallet errors.
    #[error(transparent)]
    Wallet(#[from] crate::database::WalletError),

    /// SQLite errors.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Tokio task join errors.
    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),

    /// Core wallet not configured for this wallet on a multi-wallet Core node.
    #[error(
        "Core wallet not configured for this wallet. Go to the Wallets screen and refresh to auto-detect the Core wallet association."
    )]
    CoreWalletNotConfigured,

    /// The operation's prerequisite was auto-fixed (e.g., Core wallet detected).
    /// Callers should retry the failed operation.
    #[error("{0}")]
    MustRetry(String),

    /// Duplicate identity public key — the key data already exists on the platform.
    #[error("This public key is already registered on the platform. Try a different key.")]
    DuplicateIdentityPublicKey {
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// Duplicate identity public key ID — the key hash is already taken platform-wide.
    #[error("This key hash is already registered on the platform. Try a different key.")]
    DuplicateIdentityPublicKeyId {
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// Identity public key conflicts with an existing key's unique contract bounds.
    #[error(
        "This key conflicts with an existing key bound to contract {contract_id}. Use a different key or purpose."
    )]
    IdentityPublicKeyContractBoundsConflict {
        contract_id: String,
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// Unclassified SDK error — the operation failed for an unrecognised reason.
    /// Display is implemented manually via [`sdk_error_user_message`] to inspect
    /// the source error and produce an actionable, user-friendly message.
    #[error("{}", sdk_error_user_message(source_error))]
    SdkError {
        #[source]
        source_error: Box<SdkError>,
    },
}

/// Produce a user-friendly message by inspecting the SDK error variant.
///
/// The returned text is shown in `MessageBanner` via `Display`.
/// Technical details remain available through the `#[source]` chain / `Debug`.
///
/// TODO: Expand match arms as we encounter more SDK error variants in the wild.
/// Each arm should explain *what happened* and *what the user can do*.
fn sdk_error_user_message(error: &SdkError) -> String {
    match error {
        SdkError::StateTransitionBroadcastError(e) => {
            // Known broadcast rejection that didn't match a typed consensus variant
            // above (DuplicateKey, DuplicateKeyId, ContractBoundsConflict).
            // The platform message is often the most specific info we have.
            // TODO: classify more consensus causes into dedicated TaskError variants
            //       so fewer errors reach this fallback.
            format!(
                "The platform rejected this operation: {}. Try a different approach.",
                e.message
            )
        }
        SdkError::TimeoutReached(duration, _) => {
            format!(
                "The operation did not complete within {} seconds. Please retry — it often succeeds on the second attempt.",
                duration.as_secs()
            )
        }
        SdkError::StaleNode(_) => {
            "The server you connected to is behind. Please retry — the app will pick a different server automatically.".to_string()
        }
        SdkError::DapiClientError(_) => {
            // TODO: inspect inner DapiClientError for connection refused vs TLS vs DNS.
            "Could not connect to the Dash network. Please retry in a few moments.".to_string()
        }
        SdkError::NoAvailableAddressesToRetry(_) => {
            "All Dash network servers are temporarily unreachable. Please wait a minute and retry.".to_string()
        }
        SdkError::Cancelled(_) => "The operation was cancelled.".to_string(),
        SdkError::AlreadyExists(detail) => {
            format!("This already exists on the platform: {detail}. No action needed.")
        }
        SdkError::NonceOverflow(_) => {
            "This identity has reached its maximum number of operations. Please try again later.".to_string()
        }
        SdkError::IdentityNonceNotFound(_) => {
            "The platform has not indexed this identity yet. Please retry in a few moments.".to_string()
        }
        // TODO: add arms for Protocol (consensus sub-errors), InvalidCreditTransfer,
        //       MissingDependency, Config, etc.
        _ => {
            // Fallback — the technical cause is in the #[source] chain / details panel.
            format!("Unexpected error: {}. Please try again later.", error)
        }
    }
}

impl From<String> for TaskError {
    fn from(s: String) -> Self {
        if s.contains("Wallet file not specified") {
            TaskError::CoreWalletNotConfigured
        } else {
            TaskError::Generic(s)
        }
    }
}

impl From<dashcore_rpc::Error> for TaskError {
    fn from(e: dashcore_rpc::Error) -> Self {
        if let dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(ref rpc_err)) =
            e
            && rpc_err.code == RPC_WALLET_NOT_SPECIFIED
        {
            return TaskError::CoreWalletNotConfigured;
        }
        TaskError::Generic(e.to_string())
    }
}

impl From<SdkError> for TaskError {
    fn from(error: SdkError) -> Self {
        enum ConsensusKind {
            DuplicateKey,
            DuplicateKeyId,
            ContractBoundsConflict(String),
        }

        let kind: Option<ConsensusKind> = {
            let consensus_error = match &error {
                SdkError::StateTransitionBroadcastError(broadcast_err) => {
                    broadcast_err.cause.as_ref()
                }
                SdkError::Protocol(ProtocolError::ConsensusError(ce)) => Some(ce.as_ref()),
                _ => None,
            };

            consensus_error.and_then(|ce| match ce {
                ConsensusError::StateError(StateError::DuplicatedIdentityPublicKeyStateError(
                    _,
                )) => Some(ConsensusKind::DuplicateKey),
                ConsensusError::StateError(
                    StateError::DuplicatedIdentityPublicKeyIdStateError(_),
                ) => Some(ConsensusKind::DuplicateKeyId),
                ConsensusError::StateError(
                    StateError::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError(e),
                ) => Some(ConsensusKind::ContractBoundsConflict(
                    e.contract_id().to_string(Encoding::Base58),
                )),
                _ => None,
            })
        };

        let boxed = Box::new(error);
        match kind {
            Some(ConsensusKind::DuplicateKey) => TaskError::DuplicateIdentityPublicKey {
                source_error: boxed,
            },
            Some(ConsensusKind::DuplicateKeyId) => TaskError::DuplicateIdentityPublicKeyId {
                source_error: boxed,
            },
            Some(ConsensusKind::ContractBoundsConflict(contract_id)) => {
                TaskError::IdentityPublicKeyContractBoundsConflict {
                    contract_id,
                    source_error: boxed,
                }
            }
            None => TaskError::SdkError {
                source_error: boxed,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_id_state_error::DuplicatedIdentityPublicKeyIdStateError;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_state_error::DuplicatedIdentityPublicKeyStateError;
    use dash_sdk::dpp::consensus::state::identity::identity_public_key_already_exists_for_unique_contract_bounds_error::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError;
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::platform::Identifier;

    #[test]
    fn from_string_detects_rpc_error_minus_19() {
        let err: TaskError = "JSON-RPC error: RPC error response: RpcError { code: -19, message: \"Wallet file not specified\" }".to_string().into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured),
            "Expected CoreWalletNotConfigured, got: {err:?}",
        );
    }

    #[test]
    fn from_string_passes_through_other_errors() {
        let err: TaskError = "Connection refused".to_string().into();
        assert!(
            matches!(err, TaskError::Generic(ref s) if s == "Connection refused"),
            "Expected Generic, got: {err:?}",
        );
    }

    #[test]
    fn display_message_is_user_friendly() {
        let msg = TaskError::CoreWalletNotConfigured.to_string();
        assert!(msg.contains("Wallets screen"));
        assert!(msg.contains("refresh"));
    }

    #[test]
    fn must_retry_displays_inner_message() {
        let err = TaskError::MustRetry("Auto-detected Core wallet 'mywallet'".to_string());
        assert_eq!(err.to_string(), "Auto-detected Core wallet 'mywallet'");
    }

    #[test]
    fn rpc_error_code_neg19_converts_to_core_wallet_not_configured() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -19,
            message: "Wallet file not specified".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured),
            "Expected CoreWalletNotConfigured, got: {err:?}"
        );
    }

    #[test]
    fn other_rpc_error_converts_to_generic() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -1,
            message: "Some other error".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::Generic(_)),
            "Expected Generic, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_duplicate_public_key() {
        let consensus =
            ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1, 2]));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey { .. }));
    }

    #[test]
    fn from_sdk_error_duplicate_public_key_id() {
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyIdStateError::new(vec![3]));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(matches!(
            err,
            TaskError::DuplicateIdentityPublicKeyId { .. }
        ));
    }

    #[test]
    fn from_sdk_error_contract_bounds_conflict() {
        let contract_id = Identifier::random();
        let identity_id = Identifier::random();
        let consensus = ConsensusError::from(
            IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError::new(
                identity_id,
                contract_id,
                Purpose::AUTHENTICATION,
                2,
                1,
            ),
        );
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        let expected_contract_id = contract_id.to_string(Encoding::Base58);
        assert!(
            matches!(err, TaskError::IdentityPublicKeyContractBoundsConflict { ref contract_id, .. } if *contract_id == expected_contract_id)
        );
    }

    #[test]
    fn from_sdk_error_broadcast_cause_duplicate_key() {
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1]));
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40206,
            message: "duplicate key".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = TaskError::from(sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey { .. }));
    }

    #[test]
    fn from_sdk_error_unknown_falls_back_to_broadcast_error() {
        let sdk_err = SdkError::Generic("connection timeout".to_string());
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(err, TaskError::SdkError { .. }),
            "Expected SdkError, got: {err:?}"
        );
    }
}
