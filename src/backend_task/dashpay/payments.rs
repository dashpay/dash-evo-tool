use super::encryption::decrypt_extended_public_key;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::{Value, string_encoding::Encoding};
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use platform_wallet::changeset::PlatformWalletChangeSet;
use platform_wallet::wallet::dashpay::PaymentEntry;
use std::sync::Arc;

/// Record a DashPay payment on the owner's `ManagedIdentity` and
/// queue the resulting changeset for the persister.
///
/// Phase 9b-2 entry point that replaces the previous
/// `db.save_payment` direct write. The persister catches the
/// queued `IdentityChangeSet`, picks out the
/// `dashpay_payments[tx_id]` entry, and writes it to the
/// `dashpay_payments` table on flush.
///
/// If the owner identity isn't present in the platform-wallet's
/// `IdentityManager`, this is a silent no-op (logged at `debug`) —
/// there's no in-memory state to mutate, so the persister has
/// nothing to write.
pub(crate) async fn cache_payment_via_platform_wallet(
    app_context: &AppContext,
    owner_identity: &QualifiedIdentity,
    tx_id: String,
    entry: PaymentEntry,
) {
    let pw = match app_context.platform_wallet_for_identity(owner_identity) {
        Ok(pw) => pw,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "skipping platform-wallet payment cache: no platform wallet for identity"
            );
            return;
        }
    };
    let owner_id = owner_identity.identity.id();
    let id_cs = {
        let mut state = pw.state_mut().await;
        let Some(managed) = state.identity_manager.managed_identity_mut(&owner_id) else {
            tracing::debug!(
                identity = %owner_id,
                "skipping platform-wallet payment cache: identity not in IdentityManager"
            );
            return;
        };
        managed.record_dashpay_payment(tx_id, entry)
    };
    let cs = PlatformWalletChangeSet {
        identities: Some(id_cs),
        ..Default::default()
    };
    pw.queue_persist(cs);
}

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

/// Get the next unused address index for a contact and increment it
/// Uses the database to track address indices per contact relationship
async fn get_next_address_index(
    app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: &Identifier,
) -> Result<u32, String> {
    // Get and increment the send index from database
    app_context
        .db
        .get_and_increment_send_index(identity_id, contact_id)
        .map_err(|e| format!("Failed to get address index from database: {}", e))
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
    let sender_key_index = props
        .get("senderKeyIndex")
        .and_then(|v| match v {
            Value::U32(idx) => Some(*idx),
            _ => None,
        })
        .ok_or("Missing senderKeyIndex".to_string())?;

    let recipient_key_index = props
        .get("recipientKeyIndex")
        .and_then(|v| match v {
            Value::U32(idx) => Some(*idx),
            _ => None,
        })
        .ok_or("Missing recipientKeyIndex".to_string())?;

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

    // Get our private key
    let wallets: Vec<_> = our_identity.associated_wallets.values().cloned().collect();
    let our_private_key = our_identity
        .private_keys
        .get_resolve(
            &(
                crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity,
                our_key.id(),
            ),
            &wallets,
            our_identity.network,
        )
        .map_err(|e| format!("Error resolving private key: {}", e))?
        .map(|(_, private_key)| private_key)
        .ok_or("Private key not found".to_string())?;

    // Generate ECDH shared key for decryption
    use super::encryption::generate_ecdh_shared_key;
    let shared_key = generate_ecdh_shared_key(&our_private_key, contact_key)
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
        child_number: ChildNumber::from_normal_idx(0).unwrap(),
        public_key: pubkey,
        chain_code: ChainCode::from(chain_code),
    };

    // Get the next unused address index for this contact
    let address_index =
        get_next_address_index(app_context, &our_identity.identity.id(), &contact_id).await?;

    // Derive the payment address using platform-wallet's DIP-14 derivation
    let address = platform_wallet::derive_contact_payment_address(&xpub, address_index, network)
        .map_err(|e| format!("Failed to derive payment address: {}", e))?;

    Ok((address, address_index))
}

/// Send a payment to a contact using the wallet's SPV capabilities
/// (Legacy function - preserved for reference)
#[allow(dead_code)]
pub async fn send_payment_to_contact(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount_dash: f64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    send_payment_to_contact_impl(
        app_context,
        sdk,
        from_identity,
        to_contact_id,
        amount_dash,
        memo,
    )
    .await
}

/// Send a payment to a contact using the wallet's SPV capabilities
/// This is the main implementation called from the DashPay task handler
pub async fn send_payment_to_contact_impl(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount_dash: f64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

    // Convert Dash to duffs (1 Dash = 100,000,000 duffs)
    let amount_duffs = (amount_dash * 100_000_000.0).round() as u64;

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
        subtract_fee_from_amount: false,
        memo: memo.clone(),
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

    // Record the sent payment via the platform wallet so the
    // persister writes it to `dashpay_payments` on flush (Phase 9b-2).
    cache_payment_via_platform_wallet(
        app_context,
        &from_identity,
        txid.clone(),
        platform_wallet::wallet::dashpay::PaymentEntry::new_sent(
            to_contact_id,
            amount_duffs,
            memo.clone(),
        ),
    )
    .await;

    // Convert to Dash for display
    let amount_dash = amount_duffs as f64 / 100_000_000.0;

    Ok(BackendTaskSuccessResult::DashPayPaymentSent(
        to_contact_id.to_string(Encoding::Base58),
        to_address.to_string(),
        amount_dash,
    ))
}

/// Load payment history from local database
pub async fn load_payment_history(
    app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: Option<&Identifier>,
) -> Result<Vec<PaymentRecord>, String> {
    let stored_payments = app_context
        .db
        .load_payment_history(identity_id, 100)
        .map_err(|e| format!("Failed to load payment history: {}", e))?;

    let mut records = Vec::new();
    for sp in stored_payments {
        let from_id = Identifier::from_bytes(&sp.from_identity_id)
            .map_err(|e| format!("Invalid from_identity_id: {}", e))?;
        let to_id = Identifier::from_bytes(&sp.to_identity_id)
            .map_err(|e| format!("Invalid to_identity_id: {}", e))?;

        // If a contact filter is specified, skip non-matching records
        if let Some(filter_id) = contact_id
            && from_id != *filter_id
            && to_id != *filter_id
        {
            continue;
        }

        let status = match sp.status.as_str() {
            "confirmed" => PaymentStatus::Confirmed(1),
            "failed" => PaymentStatus::Failed("Transaction failed".to_string()),
            "pending" => PaymentStatus::Pending,
            _ => PaymentStatus::Broadcast,
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

/// Update payment status after broadcast or confirmation
pub async fn update_payment_status(
    _app_context: &Arc<AppContext>,
    payment_id: &str,
    status: PaymentStatus,
    tx_id: Option<String>,
) -> Result<(), String> {
    // TODO: Update payment record in database
    tracing::debug!(
        "Would update payment {} status to {:?} with tx_id {:?}",
        payment_id,
        status,
        tx_id
    );
    Ok(())
}

/// Check if addresses have been used (for gap limit calculation)
pub async fn check_address_usage(
    _app_context: &Arc<AppContext>,
    addresses: Vec<Address>,
) -> Result<Vec<bool>, String> {
    // TODO: This would need to query Core or check transaction history
    // For now, return all as unused
    Ok(vec![false; addresses.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

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
