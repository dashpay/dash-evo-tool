use super::encryption::decrypt_extended_public_key;
use super::hd_derivation::derive_payment_address;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::consensus::encode;
use dash_sdk::dpp::dashcore::{Address, Network, Transaction, TxOut};
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
    pub to_address: Address,
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

/// Get the next unused address index for a contact
async fn get_next_address_index(
    _app_context: &Arc<AppContext>,
    _identity_id: &Identifier,
    _contact_id: &Identifier,
) -> Result<u32, String> {
    // TODO: Query local database for highest used index
    // For now, return 0 (first address)
    Ok(0)
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
    use dash_sdk::dpp::dashcore::bip32::{ChainCode, ChildNumber, ExtendedPubKey, Fingerprint};
    use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1};

    let secp = Secp256k1::new();
    let pubkey =
        PublicKey::from_slice(&public_key).map_err(|e| format!("Invalid public key: {}", e))?;

    // Note: This is a simplified reconstruction - proper implementation would preserve all fields
    let xpub = ExtendedPubKey {
        network,
        depth: 0,
        parent_fingerprint: Fingerprint::default(),
        child_number: ChildNumber::from_normal_idx(0).unwrap(),
        public_key: pubkey,
        chain_code: ChainCode::from(&chain_code),
    };

    // Get the next unused address index for this contact
    let address_index =
        get_next_address_index(app_context, &our_identity.identity.id(), &contact_id).await?;

    // Derive the payment address
    let address = derive_payment_address(&xpub, address_index)
        .map_err(|e| format!("Failed to derive payment address: {}", e))?;

    Ok((address, address_index))
}

/// Send a payment to a contact using Dash Core RPC
pub async fn send_payment_to_contact(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    from_identity: QualifiedIdentity,
    to_contact_id: Identifier,
    amount_duffs: u64,
    memo: Option<String>,
) -> Result<BackendTaskSuccessResult, String> {
    // Derive the payment address for the contact
    let (to_address, address_index) =
        derive_contact_payment_address(app_context, sdk, &from_identity, to_contact_id).await?;

    eprintln!(
        "DEBUG: Derived payment address {} (index {}) for contact {}",
        to_address,
        address_index,
        to_contact_id.to_string(Encoding::Base58)
    );

    // Convert duffs to Dash for RPC
    let amount_dash = amount_duffs as f64 / 100_000_000.0;

    // TODO: Use Dash Core RPC to send the payment
    // This would require:
    // 1. Access to the Core wallet RPC
    // 2. Ensuring the wallet has funds
    // 3. Creating and broadcasting the transaction

    // For now, we'll create a payment record for local storage
    let payment = PaymentRecord {
        id: format!(
            "{}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            to_contact_id.to_string(Encoding::Base58)
        ),
        from_identity: from_identity.identity.id(),
        to_identity: to_contact_id,
        from_address: None, // Would be filled from Core wallet
        to_address: to_address.clone(),
        amount: amount_duffs,
        tx_id: None, // Would be filled after broadcast
        memo,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        status: PaymentStatus::Pending,
        address_index,
    };

    // TODO: Store payment record in local database
    store_payment_record(app_context, &payment).await?;

    Ok(BackendTaskSuccessResult::Message(format!(
        "Payment of {} Dash to {} at address {} prepared (RPC integration pending)",
        amount_dash,
        to_contact_id.to_string(Encoding::Base58),
        to_address
    )))
}

/// Store a payment record in the local database
async fn store_payment_record(
    _app_context: &Arc<AppContext>,
    payment: &PaymentRecord,
) -> Result<(), String> {
    // TODO: Implement database storage
    // This would store the payment in a local SQLite database
    // Table schema might look like:
    // CREATE TABLE dashpay_payments (
    //     id TEXT PRIMARY KEY,
    //     from_identity BLOB NOT NULL,
    //     to_identity BLOB NOT NULL,
    //     from_address TEXT,
    //     to_address TEXT NOT NULL,
    //     amount INTEGER NOT NULL,
    //     tx_id TEXT,
    //     memo TEXT,
    //     timestamp INTEGER NOT NULL,
    //     status TEXT NOT NULL,
    //     address_index INTEGER NOT NULL
    // );

    eprintln!("DEBUG: Would store payment record: {:?}", payment);
    Ok(())
}

/// Load payment history from local database
pub async fn load_payment_history(
    _app_context: &Arc<AppContext>,
    identity_id: &Identifier,
    contact_id: Option<&Identifier>,
) -> Result<Vec<PaymentRecord>, String> {
    // TODO: Query local database for payment records
    // Filter by identity_id and optionally by contact_id

    eprintln!(
        "DEBUG: Would load payment history for identity {} with contact filter: {:?}",
        identity_id.to_string(Encoding::Base58),
        contact_id.map(|id| id.to_string(Encoding::Base58))
    );

    Ok(Vec::new())
}

/// Update payment status after broadcast or confirmation
pub async fn update_payment_status(
    _app_context: &Arc<AppContext>,
    payment_id: &str,
    status: PaymentStatus,
    tx_id: Option<String>,
) -> Result<(), String> {
    // TODO: Update payment record in database
    eprintln!(
        "DEBUG: Would update payment {} status to {:?} with tx_id {:?}",
        payment_id, status, tx_id
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

    #[test]
    fn test_payment_record_creation() {
        let from_id = Identifier::random();
        let to_id = Identifier::random();

        let payment = PaymentRecord {
            id: "test_payment".to_string(),
            from_identity: from_id,
            to_identity: to_id,
            from_address: None,
            to_address: Address::p2pkh(
                &dash_sdk::dpp::dashcore::PublicKey::from_slice(&[0x02; 33]).unwrap(),
                Network::Testnet,
            ),
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
}
