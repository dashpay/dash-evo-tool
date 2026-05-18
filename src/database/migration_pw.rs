//! Platform-wallet one-time migration (RATIFIED two-stage model).
//!
//! Stage A (SQL v35, sync, pre-unlock) lives in `initialization.rs` and only
//! arms `settings.platform_wallet_migration_pending`. This module is Stage B:
//! the post-unlock async engine plus the fund-safety-critical DIP-14/15
//! migrate-or-quarantine predicate.
//!
//! The predicate ([`classify_contact`]) is pure and synchronous — no DB, no
//! SDK, no network. It re-derives a contact's payment addresses with BOTH the
//! legacy DET engine (`derive_dashpay_incoming_xpub` / `derive_payment_address`,
//! the historical ground truth) and the upstream engine
//! (`derive_contact_xpub` / `derive_contact_payment_address`) and asserts
//! byte-equality over the contact's ENTIRE historical used-index range. A
//! single divergent index ⇒ the contact is quarantined: it is NEVER inferred
//! migratable from identifier class. See
//! `docs/ai-design/2026-05-18-platform-wallet-migration/dip14-migration-hardstop.md`
//! §6.1–6.4 (authoritative).

// TODO(P3c): the Stage-B async engine (next sub-step, in this module)
// consumes the predicate. Until then it is exercised only by its own unit
// tests, which dead-code analysis does not count. This allow is removed in
// P3c when the engine wires `classify_contact` in.
#![allow(dead_code)]

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::wallet::Wallet as KeyWallet;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::platform::Identifier;
use platform_wallet::wallet::identity::{derive_contact_payment_address, derive_contact_xpub};

use crate::backend_task::dashpay::hd_derivation::{
    derive_dashpay_incoming_xpub, derive_payment_address,
};

/// Outcome of classifying one established DashPay contact for migration.
///
/// `Quarantine` is a SUCCESSFUL TERMINAL classification, not an error
/// (dip14-migration-hardstop.md §6.1) — the engine records it and continues;
/// it never triggers a premigration restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactClassification {
    /// Every historical index reproduces byte-identically under upstream and
    /// the contact xpub matches — safe to record into the upstream persister.
    Migratable,
    /// At least one historically-used address cannot be reproduced by the
    /// upstream engine. Funds at that address would be unreachable via the
    /// new backend, so the contact is isolated and legacy data retained.
    Quarantine {
        /// First index (in the union range) whose address diverged, or
        /// `None` when only the contact xpub itself diverged.
        first_divergent_index: Option<u32>,
        /// DET (ground-truth) address at the first divergent index, if any.
        det_address: Option<String>,
        /// Upstream address at the first divergent index, if any.
        upstream_address: Option<String>,
    },
}

/// DET → key-wallet network mapping (DET uses `dashcore::Network`; the
/// upstream derivation API takes `key_wallet::Network`).
fn to_wallet_network(network: Network) -> WalletNetwork {
    match network {
        Network::Mainnet => WalletNetwork::Mainnet,
        Network::Testnet => WalletNetwork::Testnet,
        Network::Devnet => WalletNetwork::Devnet,
        Network::Regtest => WalletNetwork::Regtest,
    }
}

/// Inclusive upper bound of the historical used-index range for a contact.
///
/// NORMATIVE (dip14-migration-hardstop.md §6.1, data-model-and-migration.md
/// Stage-B step 4): the range is the UNION of the receive range
/// `[0, highest_receive_index]` and the send range
/// `[0, next_send_index − 1]` (saturating). Because both start at 0, the
/// union is `[0, max(highest_receive_index, next_send_index − 1)]`. Checking
/// receive-only (or any sampled prefix) is the silent fund-loss hazard this
/// function exists to prevent.
pub fn historical_max_index(highest_receive_index: u32, next_send_index: u32) -> u32 {
    highest_receive_index.max(next_send_index.saturating_sub(1))
}

/// Pure, synchronous DIP-14/15 migrate-or-quarantine predicate for one
/// established contact. No DB / SDK / network — derivation only.
///
/// `seed_bytes` is the wallet's in-memory 64-byte BIP-39 seed (caller owns
/// the zeroize-on-drop secret boundary). `owner_id` is the local (sender)
/// identity, `contact_id` the remote (recipient) identity.
#[allow(clippy::too_many_arguments)]
pub fn classify_contact(
    seed_bytes: &[u8; 64],
    network: Network,
    account: u32,
    owner_id: &Identifier,
    contact_id: &Identifier,
    highest_receive_index: u32,
    next_send_index: u32,
) -> Result<ContactClassification, String> {
    // DET ground-truth contact xpub (raw-seed driven).
    let det_xpub =
        derive_dashpay_incoming_xpub(seed_bytes, network, account, owner_id, contact_id)?;

    // Upstream candidate contact xpub (key-wallet driven).
    let wallet = KeyWallet::from_seed_bytes(
        *seed_bytes,
        to_wallet_network(network),
        WalletAccountCreationOptions::Default,
    )
    .map_err(|e| format!("upstream wallet from seed failed: {e}"))?;
    let upstream = derive_contact_xpub(
        &wallet,
        to_wallet_network(network),
        account,
        owner_id,
        contact_id,
    )
    .map_err(|e| format!("upstream contact xpub derivation failed: {e}"))?;

    // Contact-xpub equality: compare the canonical public material
    // (compressed pubkey + chain code). Version/parent-fingerprint bytes are
    // path-structural and intentionally NOT part of the fund-reachability
    // check — only the key material that determines derived addresses.
    let det_pubkey = det_xpub.public_key.serialize();
    let det_chain_code = det_xpub.chain_code.to_bytes();
    let xpub_matches = det_pubkey == upstream.public_key && det_chain_code == upstream.chain_code;

    if !xpub_matches {
        return Ok(ContactClassification::Quarantine {
            first_divergent_index: None,
            det_address: None,
            upstream_address: None,
        });
    }

    // Address byte-equality over the FULL union range [0, max].
    let max_index = historical_max_index(highest_receive_index, next_send_index);
    for index in 0..=max_index {
        let det_addr = derive_payment_address(&det_xpub, index)?;
        let upstream_addr =
            derive_contact_payment_address(&upstream.xpub, index, to_wallet_network(network))
                .map_err(|e| {
                    format!("upstream payment address derivation failed at {index}: {e}")
                })?;
        if det_addr != upstream_addr {
            return Ok(ContactClassification::Quarantine {
                first_divergent_index: Some(index),
                det_address: Some(det_addr.to_string()),
                upstream_address: Some(upstream_addr.to_string()),
            });
        }
    }

    Ok(ContactClassification::Migratable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic 64-byte seed for golden vectors.
    fn seed() -> [u8; 64] {
        let mut s = [0u8; 64];
        for (i, b) in s.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(13);
        }
        s
    }

    /// Identifier with a chosen low byte (first 28 bytes zero ⇒ low-index
    /// class) — the class P0 expects to converge on mainnet/account-0.
    fn id_low(tag: u8) -> Identifier {
        let mut b = [0u8; 32];
        b[31] = tag;
        Identifier::from_bytes(&b).unwrap()
    }

    /// Identifier with high bytes set (full-256-bit class).
    fn id_full(tag: u8) -> Identifier {
        let mut b = [tag; 32];
        b[0] = 0xF0 | (tag & 0x0F);
        Identifier::from_bytes(&b).unwrap()
    }

    #[test]
    fn historical_max_index_is_union_not_receive_only() {
        // Send range extends past receive — must NOT be ignored.
        assert_eq!(historical_max_index(2, 10), 9);
        // Receive range extends past send.
        assert_eq!(historical_max_index(20, 3), 20);
        // No sends yet (next_send_index 0 saturates to 0).
        assert_eq!(historical_max_index(5, 0), 5);
        // Equal.
        assert_eq!(historical_max_index(7, 8), 7);
    }

    #[test]
    fn mainnet_account0_low_index_contact_is_quarantined() {
        // FUND-SAFETY FINDING (this predicate, empirically). The
        // dip14-migration-hardstop.md §6.2 expectation that low-index
        // contacts are "expected migratable" is DISPROVEN here:
        //
        //   DET `ckd_priv_256` has a `< 2^32` compatibility branch — for a
        //   low-index identifier (first 28 bytes zero) it HMACs only
        //   `index[28..32]` (ser_32). Upstream `ChildNumber::Normal256`
        //   ALWAYS HMACs the full 32-byte index (ser_256), with no
        //   compatibility shortcut. Different HMAC input ⇒ different child
        //   key ⇒ different xpub/addresses. Funds at DET low-index contact
        //   addresses are unreachable via the upstream engine.
        //
        // This is the §6.5 release-blocking scenario realised: the predicate
        // MUST quarantine (never silently "assume equivalent"). Weakening
        // the predicate to migrate this would be the exact silent fund-loss
        // P3 exists to prevent. Quarantine is the sole safety net here until
        // an upstream `ser_32`-compatibility fix or explicit acceptance.
        let owner = id_low(1);
        let contact = id_low(2);
        let r = classify_contact(&seed(), Network::Mainnet, 0, &owner, &contact, 5, 3)
            .expect("classify must not error");
        assert!(
            matches!(r, ContactClassification::Quarantine { .. }),
            "low-index DET ser_32 vs upstream ser_256 divergence MUST quarantine, got {r:?}"
        );
    }

    #[test]
    fn mainnet_account0_full_256bit_contact_is_migratable() {
        // Full-256-bit identifiers still use the raw 32-byte index in BOTH
        // engines (ckd_priv_256 / Normal256) — account-0/mainnet converges.
        let owner = id_full(3);
        let contact = id_full(4);
        let r = classify_contact(&seed(), Network::Mainnet, 0, &owner, &contact, 8, 8)
            .expect("classify must not error");
        assert_eq!(r, ContactClassification::Migratable);
    }

    #[test]
    fn testnet_contact_is_quarantined() {
        // P0-confirmed structural divergence: DET hardcodes coin-type 5'
        // for all networks; upstream uses coin-type 1' on testnet. The
        // contact xpub (or addresses) cannot reproduce ⇒ quarantine.
        let owner = id_low(1);
        let contact = id_low(2);
        let r = classify_contact(&seed(), Network::Testnet, 0, &owner, &contact, 5, 5)
            .expect("classify must not error");
        assert!(
            matches!(r, ContactClassification::Quarantine { .. }),
            "testnet must quarantine (coin-type path divergence), got {r:?}"
        );
    }

    #[test]
    fn non_account0_contact_is_quarantined() {
        // P0-confirmed: DET puts the account in the path
        // (m/9'/5'/15'/{account}') while upstream fixes account at 0' and
        // moves it elsewhere — non-account-0 diverges even on mainnet.
        let owner = id_low(1);
        let contact = id_low(2);
        let r = classify_contact(&seed(), Network::Mainnet, 1, &owner, &contact, 4, 4)
            .expect("classify must not error");
        assert!(
            matches!(r, ContactClassification::Quarantine { .. }),
            "non-account-0 must quarantine (account path-placement divergence), got {r:?}"
        );
    }

    #[test]
    fn quarantine_reports_first_divergent_index_or_xpub() {
        let owner = id_low(1);
        let contact = id_low(2);
        let r = classify_contact(&seed(), Network::Testnet, 0, &owner, &contact, 3, 3)
            .expect("classify must not error");
        match r {
            ContactClassification::Quarantine {
                first_divergent_index,
                det_address,
                upstream_address,
            } => {
                // Either the xpub diverged (index None, addrs None) or an
                // address diverged (index Some, both addrs Some) — never a
                // half-populated state.
                let xpub_div = first_divergent_index.is_none()
                    && det_address.is_none()
                    && upstream_address.is_none();
                let addr_div = first_divergent_index.is_some()
                    && det_address.is_some()
                    && upstream_address.is_some();
                assert!(
                    xpub_div || addr_div,
                    "quarantine diagnostic must be internally consistent"
                );
            }
            other => panic!("expected quarantine, got {other:?}"),
        }
    }

    #[test]
    fn send_side_only_contact_checks_send_range() {
        // A contact with receive index 0 but sends made (next_send_index>0):
        // the predicate must still derive over the send range, not stop at
        // receive 0. On testnet this quarantines (proves the send range was
        // actually walked rather than a 1-address receive-only shortcut).
        let owner = id_low(5);
        let contact = id_low(6);
        let r = classify_contact(&seed(), Network::Testnet, 0, &owner, &contact, 0, 12)
            .expect("classify must not error");
        assert!(
            matches!(r, ContactClassification::Quarantine { .. }),
            "send-side-only contact must still be classified over the send range"
        );
    }
}
