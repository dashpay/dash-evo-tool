use super::encryption::decrypt_extended_public_key;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{
    PaymentDirection as StoredPaymentDirection, PaymentStatus as StoredPaymentStatus,
    validate_payment_memo,
};
use crate::model::dashpay_derivation::derive_payment_address;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::{Value, string_encoding::Encoding};
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Payment record for local storage
#[derive(Debug, Clone)]
pub struct PaymentRecord {
    pub id: String,
    pub from_identity: Identifier,
    pub to_identity: Identifier,
    pub from_address: Option<Address>,
    pub to_address: Option<Address>,
    pub amount: u64,
    pub tx_id: Option<String>,
    pub memo: Option<String>,
    pub timestamp: u64,
    pub status: PaymentStatus,
    pub address_index: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaymentStatus {
    Pending,
    Broadcast,
    Confirmed(u32), // Number of confirmations
    Failed(String),
}

/// Get the next unused address index for a contact and increment it.
///
/// Delegates to `WalletBackend::dashpay_increment_send_index`, which
/// serializes concurrent calls across the process via an internal mutex
/// so two parallel sends never receive the same index.
async fn get_next_address_index(
    app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: &Identifier,
) -> Result<u32, String> {
    let backend = app_context
        .wallet_backend()
        .map_err(|e| format!("Wallet backend not yet available: {}", e))?;
    backend
        .dashpay_increment_send_index(identity_id, contact_id)
        .map_err(|e| format!("Failed to allocate next DashPay address index: {}", e))
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum ContactRequestKeyIndexError {
    #[error("Missing senderKeyIndex")]
    MissingSender,
    #[error("Missing recipientKeyIndex")]
    MissingRecipient,
}

fn read_contact_request_key_indices(
    properties: &BTreeMap<String, Value>,
) -> Result<(u32, u32), ContactRequestKeyIndexError> {
    let sender = properties
        .get("senderKeyIndex")
        .ok_or(ContactRequestKeyIndexError::MissingSender)?
        .to_integer::<u32>()
        .map_err(|_| ContactRequestKeyIndexError::MissingSender)?;
    let recipient = properties
        .get("recipientKeyIndex")
        .ok_or(ContactRequestKeyIndexError::MissingRecipient)?
        .to_integer::<u32>()
        .map_err(|_| ContactRequestKeyIndexError::MissingRecipient)?;
    Ok((sender, recipient))
}

/// Derive a payment address for a contact from their encrypted extended public key
pub async fn derive_contact_payment_address(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    our_identity: &QualifiedIdentity,
    contact_id: Identifier,
) -> Result<(Address, u32), String> {
    // Fetch the contact request from the contact to us (they sent us their encrypted xpub)
    let dashpay_contract = app_context.dashpay_contract.clone();

    let mut query = DocumentQuery::new(dashpay_contract.clone(), "contactRequest")
        .map_err(|e| format!("Failed to create query: {}", e))?;

    query = query
        .with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(contact_id.to_buffer()),
        })
        .with_where(WhereClause {
            field: "toUserId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(our_identity.identity.id().to_buffer()),
        });
    query.limit = 1;

    let results = Document::fetch_many(sdk, query)
        .await
        .map_err(|e| format!("Failed to fetch contact request: {}", e))?;

    let (_doc_id, doc) = results.into_iter().next().ok_or_else(|| {
        format!(
            "No contact request found from {}",
            contact_id.to_string(Encoding::Base58)
        )
    })?;

    let doc = doc.ok_or_else(|| "Contact request document is null".to_string())?;

    // Get properties from the document - handle the Document enum properly
    let props = match &doc {
        Document::V0(doc_v0) => doc_v0.properties(),
    };

    // Get the encrypted extended public key
    let encrypted_xpub = props
        .get("encryptedPublicKey")
        .and_then(|v| v.as_bytes())
        .ok_or("Missing encryptedPublicKey in contact request".to_string())?;

    // Get key indices for decryption
    let (sender_key_index, recipient_key_index) =
        read_contact_request_key_indices(props).map_err(|error| error.to_string())?;

    // Get our private key for decryption
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

    let our_key = our_identity
        .identity
        .public_keys()
        .values()
        .find(|k| k.id() == recipient_key_index)
        .ok_or_else(|| format!("Key with index {} not found", recipient_key_index))?;

    // Get the contact's public key
    use dash_sdk::platform::Fetch;

    let contact_identity = dash_sdk::dpp::identity::Identity::fetch(sdk, contact_id)
        .await
        .map_err(|e| format!("Failed to fetch contact identity: {}", e))?
        .ok_or("Contact identity not found".to_string())?;

    let contact_key = contact_identity
        .public_keys()
        .values()
        .find(|k| k.id() == sender_key_index)
        .ok_or_else(|| format!("Contact key with index {} not found", sender_key_index))?;

    // Resolve our private key through the JIT chokepoint (no parked-seed read).
    let our_private_key = our_identity
        .resolve_private_key_bytes(our_key)
        .await
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Private key not found".to_string())?;

    // Generate ECDH shared key for decryption
    use super::encryption::generate_ecdh_shared_key;
    let shared_key = generate_ecdh_shared_key(&our_private_key[..], contact_key)
        .map_err(|e| format!("Failed to generate shared key: {}", e))?;

    // Decrypt the extended public key
    let (_parent_fingerprint, chain_code, public_key) =
        decrypt_extended_public_key(encrypted_xpub, &shared_key)
            .map_err(|e| format!("Failed to decrypt extended public key: {}", e))?;

    // Reconstruct the ExtendedPubKey
    let network = app_context.network;

    // Create extended public key from components
    // This is simplified - in production you'd properly reconstruct with all fields
    use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1};
    use dash_sdk::dpp::key_wallet::bip32::{ChainCode, ChildNumber, ExtendedPubKey, Fingerprint};

    let _secp = Secp256k1::new();
    let pubkey =
        PublicKey::from_slice(&public_key).map_err(|e| format!("Invalid public key: {}", e))?;

    // Note: This is a simplified reconstruction - proper implementation would preserve all fields
    let xpub = ExtendedPubKey {
        network,
        depth: 0,
        parent_fingerprint: Fingerprint::default(),
        child_number: ChildNumber::from_normal_idx(0)
            .expect("invariant: BIP32 child index 0 is below 2^31"),
        public_key: pubkey,
        chain_code: ChainCode::from(chain_code),
    };

    // Get the next unused address index for this contact
    let address_index =
        get_next_address_index(app_context, &our_identity.identity.id(), &contact_id).await?;

    // Derive the payment address
    let address = derive_payment_address(&xpub, address_index)
        .map_err(|e| format!("Failed to derive payment address: {}", e))?;

    Ok((address, address_index))
}

/// Send a payment to a contact using the wallet's SPV capabilities.
///
/// `amount_duffs` is the amount in duffs (1 DASH = 100,000,000 duffs); the UI
/// converts from user input at its edge, so no floating-point value crosses
/// this boundary.
pub async fn send_payment_to_contact(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount_duffs: u64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    if let Some(memo) = memo.as_deref() {
        validate_payment_memo(memo).map_err(|source| TaskError::DashPayMemoTooLong { source })?;
    }

    // Get a wallet from the identity's associated wallets
    let wallet = from_identity
        .associated_wallets
        .values()
        .next()
        .ok_or(TaskError::WalletNotFound)?
        .clone();

    // Check wallet is unlocked
    {
        let wallet_guard = wallet.read()?;
        if !wallet_guard.is_open() {
            return Err(TaskError::WalletLocked);
        }
    }

    // Derive the payment address for the contact from their encrypted extended public key
    let (to_address, address_index) =
        derive_contact_payment_address(app_context, sdk, &from_identity, to_contact_id)
            .await
            .map_err(|e| TaskError::EncryptionError { detail: e })?;

    tracing::info!(
        "Derived DashPay payment address {} (index {}) for contact {}",
        to_address,
        address_index,
        to_contact_id.to_string(Encoding::Base58)
    );

    // Build the payment request
    let request = WalletPaymentRequest {
        recipients: vec![PaymentRecipient {
            address: to_address.to_string(),
            amount_duffs,
        }],
        override_fee: None,
    };

    // Send the payment using the existing wallet infrastructure
    let result = app_context
        .run_core_task(CoreTask::SendWalletPayment {
            wallet: wallet.clone(),
            request,
        })
        .await?;

    // Extract txid from result
    let txid = match &result {
        BackendTaskSuccessResult::WalletPayment { txid, .. } => txid.clone(),
        _ => "unknown".to_string(),
    };

    // Store payment record in local database
    let payment = PaymentRecord {
        id: format!(
            "{}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            to_contact_id.to_string(Encoding::Base58)
        ),
        from_identity: from_identity.identity.id(),
        to_identity: to_contact_id,
        from_address: None,
        to_address: Some(to_address.clone()),
        amount: amount_duffs,
        tx_id: Some(txid.clone()),
        memo: memo.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        status: PaymentStatus::Broadcast,
        address_index,
    };

    // Log payment details for debugging
    tracing::debug!(
        "Storing DashPay payment record: id={}, from={}, to={}, amount={}",
        payment.id,
        payment.from_identity.to_string(Encoding::Base58),
        payment.to_identity.to_string(Encoding::Base58),
        payment.amount
    );

    // Mirror the outgoing payment through the WalletBackend adapter so the
    // upstream `ManagedIdentity` records it and the timestamp sidecar
    // reflects when DET broadcast it.
    mirror_sent_payment_to_backend(
        app_context,
        &from_identity.identity.id(),
        &txid,
        to_contact_id,
        amount_duffs,
        memo.as_deref(),
    )
    .await;

    Ok(BackendTaskSuccessResult::DashPayPaymentSent(
        to_contact_id.to_string(Encoding::Base58),
        to_address.to_string(),
        amount_duffs,
    ))
}

/// Load payment history via the `WalletBackend` DashPay adapter — the
/// upstream-backed source of truth post-D4c. The local DET cache is no
/// longer consulted.
pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: Option<&Identifier>,
) -> Result<Vec<PaymentRecord>, TaskError> {
    let backend = app_context.wallet_backend()?;
    let stored_payments = backend.dashpay_view().payments(identity_id).await;

    let mut records = Vec::new();
    for sp in stored_payments {
        let from_id = Identifier::from_bytes(&sp.from_identity_id).map_err(|_| {
            TaskError::IdentifierParsingError {
                input: hex::encode(&sp.from_identity_id),
            }
        })?;
        let to_id = Identifier::from_bytes(&sp.to_identity_id).map_err(|_| {
            TaskError::IdentifierParsingError {
                input: hex::encode(&sp.to_identity_id),
            }
        })?;

        // If a contact filter is specified, skip non-matching records
        if let Some(filter_id) = contact_id
            && from_id != *filter_id
            && to_id != *filter_id
        {
            continue;
        }

        let status = match sp.status {
            StoredPaymentStatus::Confirmed => PaymentStatus::Confirmed(1),
            StoredPaymentStatus::Failed => PaymentStatus::Failed("Transaction failed".to_string()),
            StoredPaymentStatus::Pending => PaymentStatus::Pending,
        };

        let amount = if sp.amount < 0 {
            tracing::warn!(
                "Payment {} has negative amount {}, clamping to 0",
                sp.id,
                sp.amount
            );
            0u64
        } else {
            sp.amount as u64
        };

        let timestamp = if sp.created_at < 0 {
            tracing::warn!(
                "Payment {} has negative timestamp {}, using 0",
                sp.id,
                sp.created_at
            );
            0u64
        } else {
            sp.created_at as u64
        };

        records.push(PaymentRecord {
            id: sp.id.to_string(),
            from_identity: from_id,
            to_identity: to_id,
            from_address: None,
            to_address: None,
            amount,
            tx_id: Some(sp.tx_id),
            memo: sp.memo,
            timestamp,
            status,
            address_index: 0,
        });
    }

    Ok(records)
}

/// Map a DET-local [`PaymentStatus`] onto the upstream
/// `platform_wallet` payment status the `PaymentEntry` carries.
///
/// `Broadcast` collapses to upstream `Pending`: from Core's point of
/// view a broadcast-but-unconfirmed transaction is still pending. The
/// confirmation count in `Confirmed(_)` is not represented upstream —
/// any positive count means the transaction is on-chain.
fn det_status_to_upstream(
    status: &PaymentStatus,
) -> platform_wallet::wallet::identity::types::dashpay::payment::PaymentStatus {
    use platform_wallet::wallet::identity::types::dashpay::payment::PaymentStatus as Upstream;
    match status {
        PaymentStatus::Pending | PaymentStatus::Broadcast => Upstream::Pending,
        PaymentStatus::Confirmed(_) => Upstream::Confirmed,
        PaymentStatus::Failed(_) => Upstream::Failed,
    }
}

/// Mirror a payment status transition into the upstream `ManagedIdentity`
/// and the k/v timestamp sidecar for `owner`.
///
/// The authoritative on-chain state lives with Core/SPV; this is a local
/// mirror so [`load_payment_history`] reflects the new status without a
/// refetch. Upstream stores payments keyed by `tx_id` with last-write-wins
/// semantics, so the existing entry is read, its status field updated, and
/// the whole entry written back — preserving counterparty, amount, and memo.
///
/// Best-effort: a missing wallet, an unknown payment, or a sidecar miss
/// is logged at `debug` and yields `Ok(())`. The caller has already
/// completed the authoritative action by the time this runs.
pub async fn update_payment_status(
    app_context: &Arc<AppContext>,
    owner: &Identifier,
    tx_id: &str,
    status: PaymentStatus,
) -> Result<(), String> {
    use platform_wallet::wallet::identity::types::dashpay::payment::{
        PaymentDirection, PaymentEntry,
    };

    let backend = app_context
        .wallet_backend()
        .map_err(|e| format!("Wallet backend not yet available: {}", e))?;

    // Read the existing entry so the rewrite preserves counterparty,
    // amount, and memo — upstream replaces the whole entry on record.
    let existing = backend
        .dashpay_view()
        .payments(owner)
        .await
        .into_iter()
        .find(|p| p.tx_id == tx_id);

    let Some(existing) = existing else {
        tracing::debug!(
            tx_id = %tx_id,
            owner = %owner.to_string(Encoding::Base58),
            "DashPay update_payment_status: no matching payment to update; skipping"
        );
        return Ok(());
    };

    let counterparty_bytes = if existing.payment_type == StoredPaymentDirection::Sent {
        existing.to_identity_id
    } else {
        existing.from_identity_id
    };
    let counterparty = Identifier::from_bytes(&counterparty_bytes)
        .map_err(|e| format!("Invalid counterparty identity in payment record: {}", e))?;

    let direction = if existing.payment_type == StoredPaymentDirection::Sent {
        PaymentDirection::Sent
    } else {
        PaymentDirection::Received
    };

    let entry = PaymentEntry {
        counterparty_id: counterparty,
        amount_duffs: existing.amount.max(0) as u64,
        memo: existing.memo,
        direction,
        status: det_status_to_upstream(&status),
    };

    if let Err(e) = backend
        .dashpay_record_payment(owner, tx_id.to_string(), entry)
        .await
    {
        tracing::debug!(
            tx_id = %tx_id,
            owner = %owner.to_string(Encoding::Base58),
            error = ?e,
            "DashPay update_payment_status mirror to WalletBackend failed"
        );
        return Ok(());
    }

    // A confirmation stamps `confirmed_at`; other transitions preserve the
    // existing creation stamp without touching confirmation.
    if matches!(status, PaymentStatus::Confirmed(_)) {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0);
        let created_at = if existing.created_at > 0 {
            existing.created_at
        } else {
            now_ms
        };
        if let Err(e) = backend.dashpay_set_payment_timestamps(tx_id, created_at, Some(now_ms)) {
            tracing::debug!(
                tx_id = %tx_id,
                error = ?e,
                "DashPay update_payment_status confirmation timestamp write failed"
            );
        }
    }

    Ok(())
}

/// Mirror an outgoing payment into the upstream `ManagedIdentity` and the
/// k/v timestamp sidecar so [`DashpayView::payments`] picks it up.
///
/// Best-effort: the platform-side write already succeeded by the time we
/// get here, and a local mirror miss does not break correctness.
pub(super) async fn mirror_sent_payment_to_backend(
    app_context: &Arc<AppContext>,
    owner: &Identifier,
    tx_id: &str,
    counterparty: Identifier,
    amount_duffs: u64,
    memo: Option<&str>,
) {
    use platform_wallet::wallet::identity::types::dashpay::payment::PaymentEntry;

    let Ok(backend) = app_context.wallet_backend() else {
        return;
    };

    let entry = PaymentEntry::new_sent(counterparty, amount_duffs, memo.map(str::to_string));
    if let Err(e) = backend
        .dashpay_record_payment(owner, tx_id.to_string(), entry)
        .await
    {
        tracing::debug!(
            tx_id = %tx_id,
            owner = %owner.to_string(Encoding::Base58),
            error = ?e,
            "DashPay sent-payment mirror to WalletBackend failed"
        );
        return;
    }

    let now_ms = chrono::Utc::now().timestamp_millis().max(0);
    if let Err(e) = backend.dashpay_set_payment_timestamps(tx_id, now_ms, None) {
        tracing::debug!(
            tx_id = %tx_id,
            error = ?e,
            "DashPay sent-payment timestamp sidecar write failed"
        );
    }
}

/// Mirror an incoming payment into the upstream `ManagedIdentity` and the
/// k/v timestamp sidecar. Incoming payments are recorded as
/// [`PaymentStatus::Confirmed`] because SPV only delivers them after
/// the transaction is observed on-chain.
///
/// The upstream payment map keys by an opaque `String`, so the record is keyed
/// by `(tx_id, vout)` via [`payment_storage_key`] — a transaction paying two
/// different contact outputs records both, instead of the second overwriting
/// the first. The same composite key keys the timestamp sidecar, keeping each
/// output's timestamps independent.
pub(super) async fn mirror_incoming_payment_to_backend(
    app_context: &Arc<AppContext>,
    owner: &Identifier,
    tx_id: &str,
    vout: u32,
    counterparty: Identifier,
    amount_duffs: u64,
) {
    use crate::model::dashpay::payment_storage_key;
    use platform_wallet::wallet::identity::types::dashpay::payment::PaymentEntry;

    let Ok(backend) = app_context.wallet_backend() else {
        return;
    };

    let storage_key = payment_storage_key(tx_id, vout);
    let entry = PaymentEntry::new_received(counterparty, amount_duffs, None);
    if let Err(e) = backend
        .dashpay_record_payment(owner, storage_key.clone(), entry)
        .await
    {
        tracing::debug!(
            tx_id = %tx_id,
            owner = %owner.to_string(Encoding::Base58),
            error = ?e,
            "DashPay incoming-payment mirror to WalletBackend failed"
        );
        return;
    }

    let now_ms = chrono::Utc::now().timestamp_millis().max(0);
    // Incoming arrives confirmed — same ts for `created_at` and `confirmed_at`.
    if let Err(e) = backend.dashpay_set_payment_timestamps(&storage_key, now_ms, Some(now_ms)) {
        tracing::debug!(
            tx_id = %tx_id,
            error = ?e,
            "DashPay incoming-payment timestamp sidecar write failed"
        );
    }
}

/// Check if addresses have been used (for gap limit calculation)
///
/// BLOCKED: returns all-unused. An upstream usage reader exists at
/// platform rev `35e4a2f`
/// (`PlatformWalletManager::account_address_pools_blocking` →
/// `AccountAddressInfoSnapshot::is_used`, sourced from the SPV-tracked
/// `AddressInfo.is_used()`), but it is keyed by `(wallet_id, AccountType)`,
/// not by an arbitrary address. This function receives a context-free
/// address list, so it cannot route a lookup. More fundamentally, the
/// only DashPay addresses with derivation context are the contact-SEND
/// addresses derived from the contact's xpub in
/// [`derive_contact_payment_address`]; those never live in any of our
/// managed address pools (we only register `DashpayReceivingFunds`
/// accounts for incoming payments), so even a full per-account scan
/// would correctly report them absent and yield all-unused. Returning a
/// fabricated usage flag would corrupt gap-limit math, which is
/// address-derivation-adjacent and risky — hence the honest all-unused
/// stub pending a properly-scoped reader.
pub async fn check_address_usage(
    _app_context: &Arc<AppContext>,
    addresses: Vec<Address>,
) -> Result<Vec<bool>, String> {
    Ok(vec![false; addresses.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_request_key_indices_accept_platform_integer_variants() {
        for value in [Value::I128(17), Value::U32(17), Value::I64(17)] {
            let properties = std::collections::BTreeMap::from([
                ("senderKeyIndex".to_string(), value.clone()),
                ("recipientKeyIndex".to_string(), value),
            ]);

            assert_eq!(read_contact_request_key_indices(&properties), Ok((17, 17)));
        }
    }

    fn create_test_address() -> Address {
        let pubkey_bytes = [0x02; 33];
        let pubkey = dash_sdk::dpp::dashcore::PublicKey::from_slice(&pubkey_bytes).unwrap();
        Address::p2pkh(&pubkey, dash_sdk::dpp::dashcore::Network::Testnet)
    }

    #[test]
    fn test_payment_record_creation() {
        let from_id = Identifier::random();
        let to_id = Identifier::random();

        let payment = PaymentRecord {
            id: "test_payment".to_string(),
            from_identity: from_id,
            to_identity: to_id,
            from_address: None,
            to_address: Some(create_test_address()),
            amount: 100_000_000, // 1 Dash
            tx_id: None,
            memo: Some("Test payment".to_string()),
            timestamp: 0,
            status: PaymentStatus::Pending,
            address_index: 0,
        };

        assert_eq!(payment.amount, 100_000_000);
        assert_eq!(payment.status, PaymentStatus::Pending);
    }

    #[test]
    fn test_payment_status_pending() {
        let status = PaymentStatus::Pending;
        assert_eq!(status, PaymentStatus::Pending);
    }

    #[test]
    fn test_payment_status_broadcast() {
        let status = PaymentStatus::Broadcast;
        assert_eq!(status, PaymentStatus::Broadcast);
    }

    #[test]
    fn test_payment_status_confirmed() {
        let status = PaymentStatus::Confirmed(6);
        if let PaymentStatus::Confirmed(confirmations) = status {
            assert_eq!(confirmations, 6);
        } else {
            panic!("Expected Confirmed status");
        }
    }

    #[test]
    fn test_payment_status_failed() {
        let status = PaymentStatus::Failed("Insufficient funds".to_string());
        if let PaymentStatus::Failed(msg) = status {
            assert_eq!(msg, "Insufficient funds");
        } else {
            panic!("Expected Failed status");
        }
    }

    #[test]
    fn test_payment_status_equality() {
        assert_eq!(PaymentStatus::Pending, PaymentStatus::Pending);
        assert_eq!(PaymentStatus::Broadcast, PaymentStatus::Broadcast);
        assert_eq!(PaymentStatus::Confirmed(6), PaymentStatus::Confirmed(6));
        assert_ne!(PaymentStatus::Confirmed(6), PaymentStatus::Confirmed(7));
        assert_eq!(
            PaymentStatus::Failed("error".to_string()),
            PaymentStatus::Failed("error".to_string())
        );
    }

    #[test]
    fn test_payment_record_with_tx_id() {
        let payment = PaymentRecord {
            id: "test_payment".to_string(),
            from_identity: Identifier::random(),
            to_identity: Identifier::random(),
            from_address: Some(create_test_address()),
            to_address: Some(create_test_address()),
            amount: 50_000_000, // 0.5 Dash
            tx_id: Some("abc123def456".to_string()),
            memo: None,
            timestamp: 1700000000,
            status: PaymentStatus::Broadcast,
            address_index: 5,
        };

        assert_eq!(payment.tx_id, Some("abc123def456".to_string()));
        assert_eq!(payment.status, PaymentStatus::Broadcast);
        assert_eq!(payment.address_index, 5);
        assert!(payment.from_address.is_some());
        assert!(payment.memo.is_none());
    }

    #[test]
    fn test_payment_record_amount_in_duffs() {
        // Test that we can properly handle various Dash amounts in duffs
        let test_amounts: Vec<(f64, u64)> = vec![
            (0.1, 10_000_000),              // 0.1 DASH
            (1.0, 100_000_000),             // 1 DASH
            (10.5, 1_050_000_000),          // 10.5 DASH
            (100.12345678, 10_012_345_678), // Full precision
        ];

        for (dash, expected_duffs) in test_amounts {
            let duffs = (dash * 100_000_000.0).round() as u64;
            assert_eq!(duffs, expected_duffs, "Conversion failed for {} DASH", dash);

            // Test reverse conversion
            let back_to_dash = duffs as f64 / 100_000_000.0;
            // Use approximate equality due to floating point
            assert!(
                (back_to_dash - dash).abs() < 0.00000001,
                "Reverse conversion failed for {} duffs",
                duffs
            );
        }
    }

    #[test]
    fn test_payment_record_clone() {
        let payment = PaymentRecord {
            id: "original".to_string(),
            from_identity: Identifier::random(),
            to_identity: Identifier::random(),
            from_address: None,
            to_address: Some(create_test_address()),
            amount: 100_000_000,
            tx_id: Some("tx123".to_string()),
            memo: Some("Original memo".to_string()),
            timestamp: 1700000000,
            status: PaymentStatus::Pending,
            address_index: 0,
        };

        let cloned = payment.clone();

        assert_eq!(payment.id, cloned.id);
        assert_eq!(payment.amount, cloned.amount);
        assert_eq!(payment.status, cloned.status);
        assert_eq!(payment.memo, cloned.memo);
        assert_eq!(payment.tx_id, cloned.tx_id);
    }

    #[test]
    fn test_det_status_maps_to_upstream() {
        use platform_wallet::wallet::identity::types::dashpay::payment::PaymentStatus as Upstream;

        assert_eq!(
            det_status_to_upstream(&PaymentStatus::Pending),
            Upstream::Pending
        );
        // Broadcast-but-unconfirmed is still pending from Core's view.
        assert_eq!(
            det_status_to_upstream(&PaymentStatus::Broadcast),
            Upstream::Pending
        );
        // Any positive confirmation count means on-chain.
        assert_eq!(
            det_status_to_upstream(&PaymentStatus::Confirmed(1)),
            Upstream::Confirmed
        );
        assert_eq!(
            det_status_to_upstream(&PaymentStatus::Confirmed(99)),
            Upstream::Confirmed
        );
        assert_eq!(
            det_status_to_upstream(&PaymentStatus::Failed("dropped".into())),
            Upstream::Failed
        );
    }

    #[test]
    fn test_payment_status_debug_format() {
        // Test Debug trait implementation
        let pending = format!("{:?}", PaymentStatus::Pending);
        assert!(pending.contains("Pending"));

        let broadcast = format!("{:?}", PaymentStatus::Broadcast);
        assert!(broadcast.contains("Broadcast"));

        let confirmed = format!("{:?}", PaymentStatus::Confirmed(10));
        assert!(confirmed.contains("Confirmed"));
        assert!(confirmed.contains("10"));

        let failed = format!("{:?}", PaymentStatus::Failed("Test error".to_string()));
        assert!(failed.contains("Failed"));
        assert!(failed.contains("Test error"));
    }
}
