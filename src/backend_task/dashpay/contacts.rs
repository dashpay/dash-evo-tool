use crate::backend_task::dashpay::errors::DashPayError;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dashpay::{ContactData, ContactDetails};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier};
use dash_sdk::query_types::Documents;
use platform_wallet::wallet::identity::crypto::decode_private_data;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use zeroize::Zeroizing;

// DashPay contract ID from the platform repo
pub const DASHPAY_CONTRACT_ID: [u8; 32] = [
    162, 161, 180, 172, 111, 239, 34, 234, 42, 26, 104, 232, 18, 54, 68, 179, 87, 135, 95, 107, 65,
    44, 24, 16, 146, 129, 193, 70, 231, 178, 113, 188,
];

pub async fn get_dashpay_contract(sdk: &Sdk) -> Result<Arc<DataContract>, String> {
    let contract_id = Identifier::from_bytes(&DASHPAY_CONTRACT_ID).map_err(|e| e.to_string())?;
    DataContract::fetch(sdk, contract_id)
        .await
        .map_err(|e| format!("Failed to fetch DashPay contract: {}", e))?
        .ok_or_else(|| "DashPay contract not found".to_string())
        .map(Arc::new)
}

/// Derive the DIP-0015 contactInfo encryption keys for `identity`, fetching
/// the wallet's HD seed just-in-time through the [`SecretAccess`] chokepoint.
///
/// The seed is obtained per-operation via
/// [`SecretAccess::with_secret`](crate::wallet_backend::SecretAccess::with_secret)
/// keyed by the identity's DashPay wallet seed hash, and the BIP-32
/// derivation runs inside the closure through the shared
/// [`derive_contact_info_encryption_keys`](crate::wallet_backend::derive_contact_info_encryption_keys)
/// helper — the raw seed never enters this layer.
#[allow(clippy::type_complexity)]
async fn derive_contact_info_keys(
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    derivation_index: u32,
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError> {
    let seed_hash = identity
        .dashpay_wallet_seed_hash()
        .ok_or(TaskError::ContactWalletSeedUnavailable)?;
    let network = identity.network;

    app_context
        .wallet_backend()?
        .secret_access()
        .with_secret(
            &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
            |plaintext| {
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                crate::wallet_backend::derive_contact_info_encryption_keys(
                    seed,
                    network,
                    derivation_index,
                )
            },
        )
        .await
}

/// Decrypt toUserId using AES-256-ECB as specified by DIP-0015.
///
/// DIP-0015 mandates ECB mode for encToUserId encryption because:
/// 1. The toUserId is derived from SHA256, making it appear random (no patterns)
/// 2. Keys are never reused (unique per contact via hardened BIP32 derivation)
/// 3. The data is fixed-size (32 bytes = exactly 2 AES blocks)
///
/// These properties eliminate typical ECB vulnerabilities (pattern leakage).
/// See: https://github.com/dashpay/dips/blob/master/dip-0015.md
fn decrypt_to_user_id(encrypted: &[u8], key: &[u8; 32]) -> Result<[u8; 32], String> {
    use aes_gcm::aes::Aes256;
    use aes_gcm::aes::cipher::{BlockCipherDecrypt, KeyInit};

    let mut decrypted: [u8; 32] = encrypted
        .try_into()
        .map_err(|_| "Invalid encrypted user ID length".to_string())?;

    let cipher = Aes256::new(key.into());

    // The 32-byte ciphertext is exactly two 16-byte AES blocks; decrypt each in place (ECB).
    let (blocks, _) = decrypted.as_chunks_mut::<16>();
    for block in blocks {
        cipher.decrypt_block(block.into());
    }

    Ok(decrypted)
}

// Helper function to decrypt private data using AES-256-CBC
pub(super) fn decrypt_private_data(
    encrypted_data: &[u8],
    key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    use cbc::cipher::BlockModeDecrypt;
    use cbc::cipher::KeyIvInit;
    use cbc::cipher::block_padding::Pkcs7;
    type Aes256CbcDec = cbc::Decryptor<aes_gcm::aes::Aes256>;

    // Extract IV and ciphertext
    let (iv, ciphertext) = encrypted_data
        .split_first_chunk::<16>()
        .ok_or_else(|| "Encrypted data too short (no IV)".to_string())?;

    // Decrypt
    let cipher = Aes256CbcDec::new(key.into(), iv.into());

    let mut buffer = ciphertext.to_vec();
    let decrypted = cipher
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("Decryption failed: {:?}", e))?;

    Ok(decrypted.to_vec())
}

/// The nickname, note and visibility the owner stored for each contact in
/// `contactInfo` documents sealed under the legacy key root, read from every
/// page of the owner's documents.
///
/// Pages are ordered by `$updatedAt`, oldest first, and a later document for
/// a contact replaces an earlier one, so the newest document wins. The test
/// seam over [`stored_contact_details`]'s legacy half.
#[cfg(test)]
pub(super) async fn load_contact_details<P, PF, D, DF>(
    query: DocumentQuery,
    fetch_page: P,
    derive_keys: D,
) -> Result<HashMap<Identifier, ContactData>, TaskError>
where
    P: FnMut(DocumentQuery) -> PF,
    PF: Future<Output = Result<Documents, TaskError>>,
    D: FnMut(u32) -> DF,
    DF: Future<Output = Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError>>,
{
    let documents = fetch_contact_info_documents(query, fetch_page).await?;
    legacy_details_from(&documents, derive_keys).await
}

/// Every page of the owner's `contactInfo` documents, oldest first by
/// `$updatedAt`.
async fn fetch_contact_info_documents<P, PF>(
    query: DocumentQuery,
    fetch_page: P,
) -> Result<Vec<Document>, TaskError>
where
    P: FnMut(DocumentQuery) -> PF,
    PF: Future<Output = Result<Documents, TaskError>>,
{
    let query = query.with_order_by(OrderClause {
        field: "$updatedAt".to_string(),
        ascending: true,
    });
    let documents =
        super::contact_info::fetch_all_contact_info_documents(query, fetch_page).await?;
    Ok(documents.into_values().flatten().collect())
}

/// The details `documents` (oldest first) hold under the legacy key root;
/// a later document for a contact replaces an earlier one. A document that
/// does not open is skipped.
///
/// # Errors
///
/// The keys could not be derived for a reason that holds for every
/// document, such as a cancelled password prompt.
async fn legacy_details_from<D, DF>(
    documents: &[Document],
    mut derive_keys: D,
) -> Result<HashMap<Identifier, ContactData>, TaskError>
where
    D: FnMut(u32) -> DF,
    DF: Future<Output = Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), TaskError>>,
{
    let mut contact_info_map: HashMap<Identifier, ContactData> = HashMap::new();
    for doc in documents {
        let props = doc.properties();
        let Some(deriv_idx) = props
            .get("derivationEncryptionKeyIndex")
            .and_then(|value| value.to_integer::<u32>().ok())
        else {
            continue;
        };
        let (enc_user_id_key, private_data_key) = match derive_keys(deriv_idx).await {
            Ok(keys) => keys,
            // The document's own index cannot be derived.
            Err(TaskError::ContactKeyDerivationFailed { .. }) => continue,
            Err(error) => return Err(error),
        };
        let Some(Value::Bytes(enc_user_id)) = props.get("encToUserId") else {
            continue;
        };
        let Ok(decrypted_id) = decrypt_to_user_id(enc_user_id, &enc_user_id_key) else {
            continue;
        };
        let Ok(contact_id) = Identifier::from_bytes(&decrypted_id) else {
            tracing::warn!(
                "Failed to parse decrypted contact ID (length {}), skipping contact info entry",
                decrypted_id.len()
            );
            continue;
        };
        // The DIP-15 codec platform-wallet writes with; an unreadable payload
        // shows as no stored details.
        let private_data = match props.get("privateData") {
            Some(Value::Bytes(encrypted)) => decrypt_private_data(encrypted, &private_data_key)
                .ok()
                .and_then(|plaintext| decode_private_data(&plaintext).ok()),
            _ => None,
        }
        .unwrap_or_default();

        contact_info_map.insert(
            contact_id,
            ContactData {
                identity_id: contact_id,
                account_reference: private_data.accepted_accounts.first().copied().unwrap_or(0),
                nickname: private_data.alias_name,
                note: private_data.note,
                is_hidden: private_data.display_hidden,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            },
        );
    }

    Ok(contact_info_map)
}

/// The details the owner's stored `contactInfo` documents hold.
pub(super) struct StoredContactDetails {
    /// From DIP-0015 documents platform-wallet can read, newest per contact;
    /// an error when the seed could not be used to read them. A save must not
    /// guess past that error; the contacts list may.
    pub(super) published: Result<BTreeMap<Identifier, ContactDetails>, TaskError>,
    /// From documents older versions of this app wrote under the legacy
    /// key root, newest per contact; an error as for `published`.
    pub(super) legacy: Result<HashMap<Identifier, ContactData>, TaskError>,
}

/// Read every `contactInfo` document the owner stored, both as DIP-0015
/// documents and as legacy-key ones.
///
/// platform-wallet does not read DIP-0015 documents back for a wallet loaded
/// without its seed, which is how wallets load after a restart, so they are
/// opened here with platform-wallet's keys and codec.
pub(super) async fn stored_contact_details(
    app_context: &Arc<AppContext>,
    sdk: &Sdk,
    identity: &QualifiedIdentity,
) -> Result<StoredContactDetails, TaskError> {
    let query = DocumentQuery::new(app_context.dashpay_contract.clone(), "contactInfo")
        .map_err(|e| DashPayError::QueryCreation {
            query_target: "DashPay contactInfo",
            source: Box::new(e.into()),
        })?
        .with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: WhereOperator::Equal,
            value: Value::Identifier(identity.identity.id().to_buffer()),
        });
    let documents = fetch_contact_info_documents(query, |query| async move {
        Document::fetch_many(sdk, query)
            .await
            .map_err(TaskError::from)
    })
    .await?;
    let published = match app_context.wallet_backend() {
        Ok(backend) => {
            backend
                .dashpay_open_contact_infos(identity, &documents)
                .await
        }
        Err(_) => Ok(BTreeMap::new()),
    };
    // Without a wallet seed no legacy key exists, so no document opens.
    let legacy = if identity.dashpay_wallet_seed_hash().is_some() {
        legacy_details_from(&documents, |derivation_index| {
            derive_contact_info_keys(app_context, identity, derivation_index)
        })
        .await
    } else {
        Ok(HashMap::new())
    };
    Ok(StoredContactDetails { published, legacy })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer vectors generated independently with Python
    /// `cryptography` 46 (OpenSSL). They pin DIP-15 interoperability with other
    /// DashPay clients across `cbc` / `aes` / `aes-gcm` bumps.
    const GOLDEN_KEY: [u8; 32] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f,
    ];

    #[test]
    fn decrypt_private_data_opens_golden_blob() {
        // IV (10..1f) ‖ AES-256-CBC-PKCS7(GOLDEN_KEY, IV, "DashPay private data!").
        let blob = hex::decode(
            "101112131415161718191a1b1c1d1e1f\
             faea994a146288d4cdeb1833de096de0dbc32bac4771e7d99338f76072f8700f",
        )
        .expect("blob hex");
        let plaintext = decrypt_private_data(&blob, &GOLDEN_KEY).expect("golden blob must decrypt");
        assert_eq!(plaintext, b"DashPay private data!");
    }

    #[test]
    fn decrypt_to_user_id_opens_golden_ecb_vector() {
        // AES-256-ECB(GOLDEN_KEY, 40..5f).
        let encrypted =
            hex::decode("a37edf3f975abaef937b62c78d5bb157974b412738e50f45c7f9db25413f274b")
                .expect("ciphertext hex");
        let expected: [u8; 32] = std::array::from_fn(|i| 0x40 + i as u8);
        assert_eq!(
            decrypt_to_user_id(&encrypted, &GOLDEN_KEY).expect("decrypt"),
            expected
        );
    }
}
