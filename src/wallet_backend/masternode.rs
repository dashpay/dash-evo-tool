//! Masternode key discovery and credit withdrawal on [`WalletBackend`].
//!
//! A wallet that registered a masternode holds its owner, voting and payout
//! keys in its DIP-3 provider accounts (`m/9'/coin'/3'/{1',2'}/index`) and its
//! funding accounts. The upstream `platform-wallet` decides which wallet key
//! matches which masternode key and how a withdrawal with it is signed and
//! routed; this module only marshals DET types to and from it:
//!
//! - [`WalletBackend::find_masternode_wallet_keys`] finds the wallet-held
//!   keys for a node's key hashes, seedlessly, as derivation references —
//!   no private key is derived or stored.
//! - [`WalletBackend::masternode_withdraw`] withdraws a masternode's credits
//!   through `PlatformWallet::masternode_withdraw`, signing with the wallet
//!   seed opened just-in-time through the secret chokepoint.
//!
//! Upstream's key lookups take the wallet-manager lock with `blocking_read`,
//! so every one of them runs on the blocking pool, never on an async worker,
//! and never while a DET lock is held.

use std::sync::Arc;

use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress, SyncState};
use dash_sdk::dpp::dashcore::hashes::{Hash, hash160};
use dash_sdk::dpp::dashcore::{Address, Network, PubkeyHash, ScriptBuf};
use dash_sdk::dpp::key_wallet::account::{AccountType, ManagedAccountTrait};
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::platform::Identifier;
use platform_wallet::masternode::MasternodeListSummary;
use platform_wallet::wallet::ProviderKeyKind;
use platform_wallet::wallet::masternode_withdrawal::{
    MasternodeWithdrawalKey, MasternodeWithdrawalKeys, MasternodeWithdrawalRequest,
};
use platform_wallet::wallet::platform_wallet::PlatformWallet;
use platform_wallet::{PlatformWalletError, SpvRuntime};

use super::{DetSigner, WalletBackend};
use crate::backend_task::error::TaskError;
use crate::model::masternode_keys::{MasternodeKeyHashes, MasternodeWithdrawalRole};
use crate::model::qualified_identity::encrypted_key_storage::WalletDerivationPath;
use crate::model::wallet::WalletSeedHash;

/// The fewest voting-key indices compared per wallet; a deeper voting pool is
/// scanned in full. Matches upstream's owner-key scan floor
/// (`PROVIDER_KEY_WINDOW`, private there).
const VOTING_KEY_SCAN_WINDOW: u32 = 20;

/// Where this device's wallets hold a masternode's keys. Each role is found
/// independently, so the roles may sit in different wallets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MasternodeWalletKeys {
    pub owner: Option<WalletDerivationPath>,
    pub payout: Option<WalletDerivationPath>,
    pub voting: Option<WalletDerivationPath>,
}

impl MasternodeWalletKeys {
    /// Whether every key `hashes` asks for has been found.
    fn covers(&self, hashes: &MasternodeKeyHashes) -> bool {
        (hashes.owner.is_none() || self.owner.is_some())
            && (hashes.payout.is_none() || self.payout.is_some())
            && (hashes.voting.is_none() || self.voting.is_some())
    }
}

/// One masternode credit withdrawal, in DET terms.
#[derive(Debug, Clone)]
pub struct MasternodeWithdrawal {
    /// The masternode's identity id (the display-order ProTxHash).
    pub identity_id: Identifier,
    /// The node's key hashes, as its identity carries them.
    pub key_hashes: MasternodeKeyHashes,
    /// Which wallet key signs.
    pub role: MasternodeWithdrawalRole,
    /// Credits to withdraw.
    pub amount_credits: u64,
    /// Destination for a payout-key withdrawal; `None` pays the registered
    /// payout address. Must be `None` for an owner-key withdrawal.
    pub destination: Option<Address>,
    /// The owner key's provider index as DET stored it, for an owner-key
    /// withdrawal. Upstream verifies it before use, so an owner key above
    /// its default scan window still resolves in a restored wallet.
    pub owner_key_index_hint: Option<u32>,
}

impl WalletBackend {
    /// The voting key hash the masternode list records for the node whose
    /// identity is `identity_id`, or `None` while the list is not current
    /// (see [`current_voting_key_hash`]) or does not list the node.
    ///
    /// Even a list that passes the freshness check can lag the chain, so a
    /// stored voting key that differs from this hash is only flagged to the
    /// user, never deleted automatically: the user removes it from the node's
    /// page (`IdentityTask::RemoveRetiredVotingKey`).
    pub async fn masternode_voting_key_hash(&self, identity_id: Identifier) -> Option<[u8; 20]> {
        current_voting_key_hash(self.inner.pwm.spv(), identity_id).await
    }

    /// Find which of this device's registered wallets hold the masternode
    /// keys behind `hashes`. Seedless: only public keys are derived and
    /// compared; the result names wallet derivation paths, never key bytes.
    ///
    /// Wallets are scanned in seed-hash order and the first wallet holding a
    /// role wins it.
    pub async fn find_masternode_wallet_keys(
        &self,
        hashes: MasternodeKeyHashes,
    ) -> Result<MasternodeWalletKeys, TaskError> {
        let mut found = MasternodeWalletKeys::default();
        if hashes.is_empty() {
            return Ok(found);
        }
        // Snapshot the registered wallets; the guard drops before any await.
        let registered: Vec<_> = self
            .inner
            .id_map
            .read()?
            .iter()
            .map(|(seed_hash, wallet_id)| (*seed_hash, *wallet_id))
            .collect();

        for (seed_hash, wallet_id) in registered {
            if found.covers(&hashes) {
                break;
            }
            let Some(wallet) = self.inner.pwm.get_wallet(&wallet_id).await else {
                tracing::warn!(
                    ?seed_hash,
                    ?wallet_id,
                    "Could not load this wallet to look for masternode keys; trying the next wallet"
                );
                continue;
            };
            let held = match tokio::task::spawn_blocking(move || wallet_held_keys(&wallet, &hashes))
                .await?
            {
                Ok(held) => held,
                Err(error) => {
                    tracing::warn!(
                        ?seed_hash,
                        ?error,
                        "Could not look for masternode keys in this wallet; trying the next wallet"
                    );
                    continue;
                }
            };
            let at = |derivation_path| WalletDerivationPath {
                wallet_seed_hash: seed_hash,
                derivation_path,
            };
            if found.owner.is_none() {
                found.owner = held.owner.map(at);
            }
            if found.payout.is_none() {
                found.payout = held.payout.map(at);
            }
            if found.voting.is_none() {
                found.voting = held.voting.map(at);
            }
        }
        Ok(found)
    }

    /// Withdraw a masternode's credits with a key held by the wallet
    /// `seed_hash`, through upstream's masternode withdrawal: it re-resolves
    /// the key, checks it against the identity, enforces the owner-key payout
    /// rule and refuses to broadcast on any mismatch. Returns the identity's
    /// remaining balance in credits.
    pub async fn masternode_withdraw(
        &self,
        seed_hash: &WalletSeedHash,
        withdrawal: MasternodeWithdrawal,
    ) -> Result<u64, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let hashes = withdrawal.key_hashes;
        let hint = withdrawal.owner_key_index_hint;
        let lookup = Arc::clone(&wallet);
        let keys = tokio::task::spawn_blocking(move || withdrawal_keys(&lookup, &hashes, hint))
            .await?
            .map_err(|error| map_masternode_withdraw_error(*error))?;
        let request = withdrawal_request(&withdrawal);

        self.inner
            .secret_access
            .with_secret_session(&Self::hd_scope(seed_hash), async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                wallet
                    .masternode_withdraw(request, &keys, &signer)
                    .await
                    .map_err(map_masternode_withdraw_error)
            })
            .await
    }
}

/// The two SPV reads behind [`current_voting_key_hash`]; a seam so a test
/// can move the tip between them.
trait MasternodeListReads {
    async fn sync_progress(&self) -> Option<SyncProgress>;
    async fn masternode_list_summaries(&self) -> Option<Vec<MasternodeListSummary>>;
}

impl MasternodeListReads for SpvRuntime {
    async fn sync_progress(&self) -> Option<SyncProgress> {
        SpvRuntime::sync_progress(self).await
    }

    async fn masternode_list_summaries(&self) -> Option<Vec<MasternodeListSummary>> {
        SpvRuntime::masternode_list_summaries(self).await
    }
}

/// See [`WalletBackend::masternode_voting_key_hash`].
///
/// The list carries no height of its own, so it is bracketed by two progress
/// reads. The list only moves forward, so the one read in between is at least
/// as high as the list height `before` shows; that height must be current
/// against `before` AND `after`. A tip that moves on while the list is read
/// fails the second check, so a list it has outrun is never trusted.
///
/// TODO(upstream): ask `SpvRuntime` for one atomic, height-tagged snapshot
/// of the list together with the header and peer tips it was checked
/// against. That would replace both progress reads, and only then could a
/// voting-key mismatch be trusted enough to act on without asking the user.
async fn current_voting_key_hash(
    reads: &impl MasternodeListReads,
    identity_id: Identifier,
) -> Option<[u8; 20]> {
    let before = reads.sync_progress().await?;
    let list_height = before.masternodes().ok()?.current_height();
    if !masternode_list_is_current_at(list_height, &before) {
        return None;
    }
    let summaries = reads.masternode_list_summaries().await?;
    let after = reads.sync_progress().await?;
    if !masternode_list_is_current_at(list_height, &after) {
        return None;
    }
    let wire = wire_pro_tx_hash(identity_id);
    summaries
        .iter()
        .find(|summary| summary.pro_tx_hash == wire)
        .map(|summary| summary.voting_key_id)
}

/// Upstream's request for `withdrawal`: which wallet key signs, and where
/// the credits go.
fn withdrawal_request(withdrawal: &MasternodeWithdrawal) -> MasternodeWithdrawalRequest {
    MasternodeWithdrawalRequest {
        pro_tx_hash: wire_pro_tx_hash(withdrawal.identity_id),
        owner_key_hash: withdrawal.key_hashes.owner,
        amount_credits: withdrawal.amount_credits,
        signing_key: match withdrawal.role {
            MasternodeWithdrawalRole::Owner => MasternodeWithdrawalKey::Owner,
            MasternodeWithdrawalRole::Payout => MasternodeWithdrawalKey::Transfer,
        },
        destination: withdrawal.destination.clone(),
    }
}

/// The ProTxHash in wire order: Platform identity ids use the reversed
/// (display) order.
fn wire_pro_tx_hash(identity_id: Identifier) -> [u8; 32] {
    let mut wire = identity_id.to_buffer();
    wire.reverse();
    wire
}

/// The P2PKH payout script whose key hash is `payout`.
fn p2pkh_script(payout: &[u8; 20]) -> Vec<u8> {
    ScriptBuf::new_p2pkh(&PubkeyHash::from_byte_array(*payout)).to_bytes()
}

/// Upstream errors are large; the blocking helpers below box them.
type BoxedError = Box<PlatformWalletError>;

/// Upstream's owner / payout key resolution for one wallet. Blocking.
/// `owner_key_index_hint` is the owner key's index when DET already knows
/// it; upstream checks that index first, then scans its default window.
fn withdrawal_keys(
    wallet: &PlatformWallet,
    hashes: &MasternodeKeyHashes,
    owner_key_index_hint: Option<u32>,
) -> Result<MasternodeWithdrawalKeys, BoxedError> {
    let payout_script = hashes.payout.as_ref().map(p2pkh_script);
    wallet
        .masternode_withdrawal_keys(
            hashes.owner.as_ref(),
            payout_script.as_deref(),
            owner_key_index_hint,
        )
        .map_err(Box::new)
}

/// The derivation paths of the keys one wallet holds.
struct HeldKeys {
    owner: Option<DerivationPath>,
    payout: Option<DerivationPath>,
    voting: Option<DerivationPath>,
}

/// Which of `hashes` one wallet holds. Blocking.
fn wallet_held_keys(
    wallet: &PlatformWallet,
    hashes: &MasternodeKeyHashes,
) -> Result<HeldKeys, BoxedError> {
    let (owner, payout) = if hashes.owner.is_some() || hashes.payout.is_some() {
        // Discovery has no stored owner index to offer yet.
        let keys = withdrawal_keys(wallet, hashes, None)?;
        (keys.owner_key.map(|(_, path)| path), keys.transfer_key)
    } else {
        (None, None)
    };
    let voting = match &hashes.voting {
        Some(voting) => find_voting_key(wallet, voting)?,
        None => None,
    };
    Ok(HeldKeys {
        owner,
        payout,
        voting,
    })
}

/// The DIP-3 voting-key path whose public key hashes to `voting`, derived
/// seedlessly from the wallet's provider-voting account. `None` when the
/// wallet has no such account or no key in the scan window matches.
fn find_voting_key(
    wallet: &PlatformWallet,
    voting: &[u8; 20],
) -> Result<Option<DerivationPath>, BoxedError> {
    for index in 0..voting_key_scan_bound(wallet) {
        let derived = match wallet.derive_provider_key_at_index(
            ProviderKeyKind::Voting,
            index,
            None,
            false,
        ) {
            Ok(derived) => derived,
            Err(PlatformWalletError::AddressNotFound(_)) => return Ok(None),
            Err(e) => return Err(Box::new(e)),
        };
        if hash160::Hash::hash(&derived.public_key_bytes).to_byte_array() == *voting {
            return provider_key_path(AccountType::ProviderVotingKeys, wallet.network(), index)
                .map(Some);
        }
    }
    Ok(None)
}

/// How many voting-key indices to compare: the depth of the wallet's voting
/// pool, never fewer than [`VOTING_KEY_SCAN_WINDOW`]. Blocking; the
/// wallet-manager guard drops before any key is derived.
fn voting_key_scan_bound(wallet: &PlatformWallet) -> u32 {
    let manager = wallet.wallet_manager().blocking_read();
    let highest_generated = manager
        .get_wallet_info(&wallet.wallet_id())
        .and_then(|info| info.core_wallet.accounts.provider_voting_keys.as_ref())
        .and_then(|account| {
            account
                .managed_account_type()
                .address_pools()
                .iter()
                .filter_map(|pool| pool.highest_generated)
                .max()
        });
    scan_bound(highest_generated)
}

/// The indices `0..bound` to scan for a pool whose highest generated index is
/// `highest_generated`.
fn scan_bound(highest_generated: Option<u32>) -> u32 {
    highest_generated
        .map_or(VOTING_KEY_SCAN_WINDOW, |highest| highest.saturating_add(1))
        .max(VOTING_KEY_SCAN_WINDOW)
}

/// A provider account's base path plus the non-hardened key `index` — how
/// the account's address pool derives it.
fn provider_key_path(
    account_type: AccountType,
    network: Network,
    index: u32,
) -> Result<DerivationPath, BoxedError> {
    let base = account_type
        .derivation_path(network)
        .map_err(|e| Box::new(PlatformWalletError::KeyDerivation(e.to_string())))?;
    let child = ChildNumber::from_normal_idx(index)
        .map_err(|e| Box::new(PlatformWalletError::KeyDerivation(e.to_string())))?;
    Ok(base.child(child))
}

/// Whether a masternode list built at `list_height` is current by `progress`:
/// both the header and the masternode phase synced, and `list_height` no
/// lower than the stored header tip, the best height connected peers announce
/// and the header tip the masternode phase knows of.
///
/// A phase's `Synced` alone is not enough: both stay `Synced` while they catch
/// up with new blocks, so a list cached before a restart or a sleep still says
/// `Synced` while it names keys the node has since replaced. With no peers the
/// header phase waits for connections, so an offline list is never current.
///
/// TODO(upstream): peers' announced height is the freshest tip available
/// here; a block no connected peer has announced yet is not seen. A
/// list-freshness query on `SpvRuntime` would replace this check.
fn masternode_list_is_current_at(list_height: u32, progress: &SyncProgress) -> bool {
    let (Ok(headers), Ok(masternodes)) = (progress.headers(), progress.masternodes()) else {
        return false;
    };
    headers.state() == SyncState::Synced
        && masternodes.state() == SyncState::Synced
        && list_height
            >= headers
                .tip_height()
                .max(headers.target_height())
                .max(masternodes.block_header_tip_height())
}

/// Map an upstream masternode-withdrawal error to a typed [`TaskError`].
fn map_masternode_withdraw_error(error: PlatformWalletError) -> TaskError {
    match error {
        PlatformWalletError::Sdk(sdk) => TaskError::from(sdk),
        PlatformWalletError::IdentityNotFound(_) => TaskError::IdentityNotFound,
        unconfirmed @ PlatformWalletError::MasternodeWithdrawalUnconfirmed { .. } => {
            TaskError::MasternodeWithdrawalUnconfirmed {
                source: Arc::new(unconfirmed),
            }
        }
        other => TaskError::MasternodeWithdrawalFailed {
            source: Arc::new(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::wired_test_app_context;
    use dash_sdk::dash_spv::sync::{BlockHeadersProgress, MasternodesProgress};
    use dash_sdk::dpp::dashcore::address::Payload;

    /// Whether `progress` shows its own masternode list current.
    fn masternode_list_is_current(progress: &SyncProgress) -> bool {
        progress.masternodes().is_ok_and(|masternodes| {
            masternode_list_is_current_at(masternodes.current_height(), progress)
        })
    }

    /// Sync progress with headers stored up to `header_tip` of the
    /// `peer_best` height peers announce, and the masternode list built at
    /// `list_height` while the masternode phase knows of `list_tip`.
    fn progress(
        headers_state: SyncState,
        header_tip: u32,
        peer_best: u32,
        list_state: SyncState,
        list_height: u32,
        list_tip: u32,
    ) -> SyncProgress {
        let mut headers = BlockHeadersProgress::default();
        headers.set_state(headers_state);
        headers.update_tip_height(header_tip);
        headers.update_target_height(peer_best);
        let mut masternodes = MasternodesProgress::default();
        masternodes.set_state(list_state);
        masternodes.update_current_height(list_height);
        masternodes.update_block_header_tip_height(list_tip);
        let mut progress = SyncProgress::default();
        progress.update_headers(headers);
        progress.update_masternodes(masternodes);
        progress
    }

    #[test]
    fn masternode_list_at_the_network_tip_is_current() {
        let synced = progress(
            SyncState::Synced,
            1_000,
            1_000,
            SyncState::Synced,
            1_000,
            1_000,
        );
        assert!(masternode_list_is_current(&synced));
    }

    /// Both phases stay `Synced` while they catch up with new blocks, so a
    /// list behind the tip still says `Synced`: its voting key may be one the
    /// node has since replaced.
    #[test]
    fn stale_masternode_list_is_not_current() {
        let behind_its_tip = progress(
            SyncState::Synced,
            1_010,
            1_010,
            SyncState::Synced,
            1_000,
            1_010,
        );
        assert!(!masternode_list_is_current(&behind_its_tip));
        let behind_stored_headers = progress(
            SyncState::Synced,
            1_010,
            1_010,
            SyncState::Synced,
            1_000,
            1_000,
        );
        assert!(!masternode_list_is_current(&behind_stored_headers));
        let behind_peers = progress(
            SyncState::Synced,
            1_000,
            1_010,
            SyncState::Synced,
            1_000,
            1_000,
        );
        assert!(!masternode_list_is_current(&behind_peers));
    }

    #[test]
    fn masternode_list_is_not_current_while_any_phase_syncs() {
        let list_syncing = progress(
            SyncState::Synced,
            1_000,
            1_000,
            SyncState::Syncing,
            1_000,
            1_000,
        );
        assert!(!masternode_list_is_current(&list_syncing));
        let headers_syncing = progress(
            SyncState::Syncing,
            1_000,
            1_000,
            SyncState::Synced,
            1_000,
            1_000,
        );
        assert!(!masternode_list_is_current(&headers_syncing));
        // With no peers left the header phase waits for connections.
        let offline = progress(
            SyncState::WaitingForConnections,
            1_000,
            1_000,
            SyncState::Synced,
            1_000,
            1_000,
        );
        assert!(!masternode_list_is_current(&offline));
        let mut list_only = SyncProgress::default();
        let mut masternodes = MasternodesProgress::default();
        masternodes.set_state(SyncState::Synced);
        list_only.update_masternodes(masternodes);
        assert!(!masternode_list_is_current(&list_only));
        assert!(!masternode_list_is_current(&SyncProgress::default()));
    }

    /// Scripted SPV reads: each progress read returns the next snapshot, so
    /// a test can move the tip between the reads around the list read.
    struct ScriptedReads {
        progress: std::sync::Mutex<std::collections::VecDeque<SyncProgress>>,
        list: Vec<MasternodeListSummary>,
    }

    impl MasternodeListReads for ScriptedReads {
        async fn sync_progress(&self) -> Option<SyncProgress> {
            self.progress.lock().unwrap().pop_front()
        }

        async fn masternode_list_summaries(&self) -> Option<Vec<MasternodeListSummary>> {
            Some(self.list.clone())
        }
    }

    fn listed_node(pro_tx_hash: [u8; 32], voting_key_id: [u8; 20]) -> MasternodeListSummary {
        MasternodeListSummary {
            pro_tx_hash,
            service_address: None,
            platform_http_port: None,
            operator_public_key: [0; 48],
            voting_key_id,
            platform_node_id: None,
            is_valid: true,
            is_evonode: false,
            has_extended_net_info: false,
        }
    }

    /// Looks up a listed node through progress snapshots `before` and
    /// `after` the list read.
    async fn voting_hash_across(before: SyncProgress, after: SyncProgress) -> Option<[u8; 20]> {
        let identity_id = Identifier::from([7u8; 32]);
        let reads = ScriptedReads {
            progress: std::sync::Mutex::new([before, after].into()),
            list: vec![listed_node(wire_pro_tx_hash(identity_id), [0x42; 20])],
        };
        current_voting_key_hash(&reads, identity_id).await
    }

    fn synced_at(header_tip: u32, peer_best: u32, list_height: u32) -> SyncProgress {
        progress(
            SyncState::Synced,
            header_tip,
            peer_best,
            SyncState::Synced,
            list_height,
            header_tip,
        )
    }

    #[tokio::test]
    async fn a_list_current_across_both_reads_names_the_voting_key() {
        let hash = voting_hash_across(
            synced_at(1_000, 1_000, 1_000),
            synced_at(1_000, 1_000, 1_000),
        );
        assert_eq!(hash.await, Some([0x42; 20]));
    }

    /// Progress is current at 1 000, then the headers or the peers' best
    /// height move on while the list read in between can still be the one
    /// built at 1 000: its voting key may already be replaced.
    #[tokio::test]
    async fn a_tip_that_moves_past_the_list_between_reads_names_no_voting_key() {
        let before = synced_at(1_000, 1_000, 1_000);
        assert_eq!(
            voting_hash_across(before.clone(), synced_at(1_001, 1_001, 1_000)).await,
            None,
            "headers moved on"
        );
        assert_eq!(
            voting_hash_across(before.clone(), synced_at(1_000, 1_001, 1_000)).await,
            None,
            "peers announced a newer block"
        );
        // The list caught up too, but the list read may predate that: only
        // the height known before the read is certain.
        assert_eq!(
            voting_hash_across(before, synced_at(1_001, 1_001, 1_001)).await,
            None,
            "list caught up after it was read"
        );
    }

    #[tokio::test]
    async fn a_list_that_stops_being_current_between_reads_names_no_voting_key() {
        let mut syncing = synced_at(1_000, 1_000, 1_000);
        let mut masternodes = syncing.masternodes().unwrap().clone();
        masternodes.set_state(SyncState::Syncing);
        syncing.update_masternodes(masternodes);
        assert_eq!(
            voting_hash_across(synced_at(1_000, 1_000, 1_000), syncing).await,
            None
        );
        assert_eq!(
            voting_hash_across(
                synced_at(1_000, 1_010, 1_000),
                synced_at(1_000, 1_000, 1_000)
            )
            .await,
            None,
            "not current before the read"
        );
    }

    use std::str::FromStr;
    use std::sync::LazyLock;

    /// The test wallet's seed: random per run, never committed.
    static SEED: LazyLock<[u8; 64]> = LazyLock::new(rand::random);

    /// hash160 of the compressed public key at `path` — an independent DIP-3
    /// vector computed straight from the seed, not through either wallet.
    fn key_hash_at(path: &str) -> [u8; 20] {
        DerivationPath::from_str(path)
            .unwrap()
            .derive_priv_ecdsa_for_master_seed(&*SEED, Network::Testnet)
            .unwrap()
            .to_priv()
            .public_key()
            .pubkey_hash()
            .to_byte_array()
    }

    async fn backend_with_wallet(
        dir: &std::path::Path,
    ) -> (Arc<crate::context::AppContext>, WalletSeedHash) {
        let context = wired_test_app_context(dir).await;
        let wallet =
            crate::model::wallet::Wallet::new_from_seed(*SEED, Network::Testnet, None, None)
                .unwrap();
        let seed_hash = wallet.seed_hash();
        context
            .wallet_backend()
            .unwrap()
            .register_wallet_from_seed(&seed_hash, &SEED, Some(0))
            .await
            .unwrap();
        (context, seed_hash)
    }

    /// A random testnet P2PKH address no test wallet holds.
    fn foreign_address() -> Address {
        Address::new(
            Network::Testnet,
            Payload::PubkeyHash(PubkeyHash::from_byte_array(rand::random())),
        )
    }

    fn withdrawal(
        role: MasternodeWithdrawalRole,
        destination: Option<Address>,
    ) -> MasternodeWithdrawal {
        MasternodeWithdrawal {
            identity_id: Identifier::from(rand::random::<[u8; 32]>()),
            key_hashes: MasternodeKeyHashes {
                owner: Some(rand::random()),
                payout: Some(rand::random()),
                voting: None,
            },
            role,
            amount_credits: 250_000,
            destination,
            owner_key_index_hint: Some(7),
        }
    }

    /// A restored wallet's owner key can sit above upstream's default scan
    /// window; the stored index carried as the hint still resolves it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_high_index_owner_key_resolves_through_its_stored_index() {
        let dir = tempfile::tempdir().unwrap();
        let (context, seed_hash) = backend_with_wallet(dir.path()).await;
        let wallet = context
            .wallet_backend()
            .unwrap()
            .resolve_wallet(&seed_hash)
            .await
            .unwrap();
        let hashes = MasternodeKeyHashes {
            owner: Some(key_hash_at("m/9'/1'/3'/2'/150")),
            payout: None,
            voting: None,
        };

        let (unhinted, hinted) = tokio::task::spawn_blocking(move || {
            (
                withdrawal_keys(&wallet, &hashes, None).unwrap(),
                withdrawal_keys(&wallet, &hashes, Some(150)).unwrap(),
            )
        })
        .await
        .unwrap();
        assert_eq!(unhinted.owner_key, None, "outside the default scan window");
        assert_eq!(
            hinted.owner_key,
            Some((150, DerivationPath::from_str("m/9'/1'/3'/2'/150").unwrap()))
        );
    }

    /// The owner key signs an owner withdrawal, which takes no destination;
    /// the node's owner hash and the wire-order ProTxHash go along.
    #[test]
    fn an_owner_withdrawal_asks_for_the_owner_key() {
        let owner = withdrawal(MasternodeWithdrawalRole::Owner, None);
        let request = withdrawal_request(&owner);
        assert_eq!(request.signing_key, MasternodeWithdrawalKey::Owner);
        assert_eq!(request.owner_key_hash, owner.key_hashes.owner);
        assert_eq!(request.pro_tx_hash, wire_pro_tx_hash(owner.identity_id));
        assert_eq!(request.amount_credits, owner.amount_credits);
        assert_eq!(request.destination, None);
    }

    /// The payout key signs a payout withdrawal, and its destination is
    /// passed through unchanged.
    #[test]
    fn a_payout_withdrawal_asks_for_the_transfer_key_and_keeps_its_destination() {
        let destination = foreign_address();
        let payout = withdrawal(MasternodeWithdrawalRole::Payout, Some(destination.clone()));
        let request = withdrawal_request(&payout);
        assert_eq!(request.signing_key, MasternodeWithdrawalKey::Transfer);
        assert_eq!(request.owner_key_hash, payout.key_hashes.owner);
        assert_eq!(request.destination, Some(destination));
    }

    /// An owner-key withdrawal to a chosen address goes through the wallet's
    /// own withdrawal, which refuses it before anything is broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_owner_withdrawal_to_another_address_is_refused_before_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let (context, seed_hash) = backend_with_wallet(dir.path()).await;
        let backend = context.wallet_backend().unwrap();
        backend.wallet_seeds().set_raw(&seed_hash, &SEED).unwrap();

        let owner_path = "m/9'/1'/3'/2'/3";
        let request = MasternodeWithdrawal {
            key_hashes: MasternodeKeyHashes {
                owner: Some(key_hash_at(owner_path)),
                payout: None,
                voting: None,
            },
            owner_key_index_hint: Some(3),
            ..withdrawal(MasternodeWithdrawalRole::Owner, Some(foreign_address()))
        };
        let error = backend
            .masternode_withdraw(&seed_hash, request)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                TaskError::MasternodeWithdrawalFailed { source }
                    if matches!(**source, PlatformWalletError::InvalidParameter(_))
            ),
            "unexpected error: {error:?}"
        );
    }

    /// The owner and voting keys are found at the DIP-3 provider paths with a
    /// NON-hardened key index (`m/9'/1'/3'/2'/3`, `m/9'/1'/3'/1'/5`), and the
    /// payout key at its BIP-44 receive path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finds_wallet_keys_at_dip3_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (context, seed_hash) = backend_with_wallet(dir.path()).await;
        let backend = context.wallet_backend().unwrap();

        let hashes = MasternodeKeyHashes {
            owner: Some(key_hash_at("m/9'/1'/3'/2'/3")),
            payout: Some(key_hash_at("m/44'/1'/0'/0/2")),
            voting: Some(key_hash_at("m/9'/1'/3'/1'/5")),
        };
        let found = backend.find_masternode_wallet_keys(hashes).await.unwrap();

        let at = |path: &str| {
            Some(WalletDerivationPath {
                wallet_seed_hash: seed_hash,
                derivation_path: DerivationPath::from_str(path).unwrap(),
            })
        };
        assert_eq!(found.owner, at("m/9'/1'/3'/2'/3"));
        assert_eq!(found.voting, at("m/9'/1'/3'/1'/5"));
        assert_eq!(found.payout, at("m/44'/1'/0'/0/2"));
    }

    /// A hardened provider index (what DET's legacy address bootstrap
    /// derives) is not where a DIP-3 wallet keeps the key, so a node whose
    /// owner key sits there is not reported as wallet-held; nor is a node
    /// registered with keys from another wallet.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foreign_and_hardened_keys_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let (context, _) = backend_with_wallet(dir.path()).await;
        let backend = context.wallet_backend().unwrap();

        let hashes = MasternodeKeyHashes {
            owner: Some(key_hash_at("m/9'/1'/3'/2'/3'")),
            payout: Some([0x77; 20]),
            voting: Some([0x88; 20]),
        };
        let found = backend.find_masternode_wallet_keys(hashes).await.unwrap();
        assert_eq!(found, MasternodeWalletKeys::default());
    }

    /// A voting pool deeper than the scan floor is scanned in full; a shallow
    /// or missing one still gets the floor.
    #[test]
    fn the_voting_scan_covers_the_whole_voting_pool() {
        assert_eq!(scan_bound(None), VOTING_KEY_SCAN_WINDOW);
        assert_eq!(scan_bound(Some(3)), VOTING_KEY_SCAN_WINDOW);
        assert_eq!(
            scan_bound(Some(VOTING_KEY_SCAN_WINDOW)),
            VOTING_KEY_SCAN_WINDOW + 1
        );
        assert_eq!(scan_bound(Some(44)), 45);
        assert_eq!(scan_bound(Some(u32::MAX)), u32::MAX);
    }

    #[test]
    fn wire_pro_tx_hash_reverses_the_identity_id() {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        let wire = wire_pro_tx_hash(Identifier::from(bytes));
        assert_eq!(wire[31], 1);
        assert_eq!(wire[0], 0);
    }

    #[test]
    fn unconfirmed_withdrawal_maps_to_its_own_variant() {
        let unconfirmed = PlatformWalletError::MasternodeWithdrawalUnconfirmed {
            identity_id: Identifier::from([1u8; 32]),
            amount_credits: 1000,
            reason: "timeout".to_string(),
        };
        assert!(matches!(
            map_masternode_withdraw_error(unconfirmed),
            TaskError::MasternodeWithdrawalUnconfirmed { .. }
        ));
        assert!(matches!(
            map_masternode_withdraw_error(PlatformWalletError::InvalidParameter(
                "owner key with destination".to_string()
            )),
            TaskError::MasternodeWithdrawalFailed { .. }
        ));
    }
}
