//! DashPay `contactInfo` (DIP-0015) through platform-wallet.
//!
//! platform-wallet fetches, matches, encodes and publishes `contactInfo`
//! documents itself; it asks the host only to seal and open their two
//! ciphertexts. [`ContactInfoCrypto`] answers those two calls from the wallet
//! seed the secret seam lends for one operation. The child keys come from
//! platform-wallet's own `derive_contact_info_keys`, the AES steps from the
//! `platform-encryption` crate it uses, so both sides always agree.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{
    ChildNumber, DerivationPath, ExtendedPubKey, KeyDerivationType,
};
use dash_sdk::dpp::key_wallet::wallet::Wallet;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::dpp::secp256k1::{PublicKey, SecretKey};
use dash_sdk::platform::{Document, Identifier};
use platform_wallet::error::PlatformWalletError;
use platform_wallet::wallet::identity::crypto::contact_info::{
    ContactInfoKeys, derive_contact_info_keys,
};
use platform_wallet::wallet::identity::crypto::decode_private_data;
use platform_wallet::wallet::identity::network::{
    ContactCryptoProvider, ContactInfoOpened, ContactInfoPublishOutcome, ContactInfoSealed,
    identity_auth_derivation_path_for_type,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::dashpay::ContactDetails;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::wallet_backend::{SecretScope, WalletBackend};

impl WalletBackend {
    /// Refresh the contact details platform-wallet holds for the identities
    /// of the wallet that manages `owner`. Does nothing when no wallet does.
    pub(crate) async fn dashpay_sync_contact_infos(
        &self,
        owner: &Identifier,
    ) -> Result<(), TaskError> {
        let Some(wallet) = self.find_wallet_for_identity(owner).await else {
            return Ok(());
        };
        wallet
            .identity()
            .dashpay()
            .sync_contact_infos()
            .await
            .map(|_| ())
            .map_err(|source| TaskError::WalletBackend {
                source: Arc::new(source),
            })
    }

    /// The details platform-wallet holds for each of `owner`'s established
    /// contacts. Empty when no wallet manages `owner`.
    pub(crate) async fn dashpay_contact_details(
        &self,
        owner: &Identifier,
    ) -> BTreeMap<Identifier, ContactDetails> {
        let Some(wallet) = self.find_wallet_for_identity(owner).await else {
            return BTreeMap::new();
        };
        let state = wallet.state().await;
        let Some(managed) = state.identity_manager.managed_identity(owner) else {
            return BTreeMap::new();
        };
        managed
            .dashpay()
            .established_contacts()
            .values()
            .map(|contact| {
                (
                    contact.contact_identity_id,
                    ContactDetails {
                        nickname: contact.alias.clone(),
                        note: contact.note.clone(),
                        hidden: contact.is_hidden,
                    },
                )
            })
            .collect()
    }

    /// Save `details` for `contact_id` and publish them through
    /// platform-wallet, which writes the identity's `contactInfo` document.
    ///
    /// # Errors
    ///
    /// Before platform-wallet changes anything: the identity is not managed
    /// by its DashPay wallet or has no wallet identity index
    /// ([`TaskError::ContactInfoIdentityNotFromWallet`]), or `contact_id` is
    /// not an established contact
    /// ([`TaskError::ContactInfoContactNotEstablished`]). A failed publish is
    /// [`TaskError::DashPayContactInfoWriteFailed`].
    pub(crate) async fn dashpay_set_contact_info(
        &self,
        identity: &QualifiedIdentity,
        contact_id: &Identifier,
        details: ContactDetails,
    ) -> Result<ContactInfoPublishOutcome, TaskError> {
        let owner = identity.identity.id();
        let seed_hash = identity
            .dashpay_wallet_seed_hash()
            .ok_or(TaskError::ContactInfoIdentityNotFromWallet)?;
        let wallet = self.resolve_wallet(&seed_hash).await?;
        let identity_index = {
            let state = wallet.state().await;
            let managed = state.identity_manager.managed_identity(&owner);
            contact_info_write_index(
                managed.and_then(|managed| managed.identity_index),
                managed.is_some_and(|managed| {
                    managed
                        .dashpay()
                        .established_contacts()
                        .contains_key(contact_id)
                }),
            )?
        };
        let network = identity.network;
        let identity_wallet = wallet.identity();
        let dashpay = identity_wallet.dashpay();

        self.secret_access()
            .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                let crypto = ContactInfoCrypto::new(seed, network, identity_index);
                dashpay
                    .set_contact_info_with_external_signer(
                        &owner,
                        contact_id,
                        details.nickname,
                        details.note,
                        details.hidden,
                        identity,
                        &crypto,
                    )
                    .await
                    .map_err(|source| TaskError::DashPayContactInfoWriteFailed {
                        source: Arc::new(source),
                    })
            })
            .await
    }
}

#[cfg(test)]
impl WalletBackend {
    /// Set the details platform-wallet holds for `contact_id`, as a sync from
    /// another device would, without publishing anything.
    pub(crate) async fn set_contact_details_for_tests(
        &self,
        seed_hash: &crate::model::wallet::WalletSeedHash,
        owner: &Identifier,
        contact_id: &Identifier,
        details: ContactDetails,
    ) {
        use platform_wallet::wallet::identity::crypto::ContactInfoPrivateData;
        let wallet = self.resolve_wallet(seed_hash).await.expect("wallet");
        let persister = wallet.persister().clone();
        let mut manager = wallet.wallet_manager().write().await;
        let applied = manager
            .get_wallet_info_mut(&wallet.wallet_id())
            .and_then(|info| info.identity_manager.managed_identity_mut(owner))
            .expect("managed identity")
            .set_contact_metadata(
                contact_id,
                ContactInfoPrivateData {
                    alias_name: details.nickname,
                    note: details.note,
                    display_hidden: details.hidden,
                    accepted_accounts: Vec::new(),
                },
                &persister,
            )
            .expect("metadata saved");
        assert!(applied, "the contact must be established");
    }
}

impl WalletBackend {
    /// The details `identity`'s DIP-0015 `contactInfo` documents hold, read
    /// with platform-wallet's keys and codec; `documents` are oldest first and
    /// the newest per contact wins. Documents that do not open — legacy-key
    /// ones, or another client's — are skipped. Empty when the identity has
    /// no wallet or wallet identity index, so none can be DIP-0015 ones.
    ///
    /// # Errors
    ///
    /// The identity's wallet is not loaded or not registered
    /// ([`TaskError::WalletNotLoaded`], [`TaskError::WalletLockedNotLoaded`],
    /// [`TaskError::WalletStateInconsistent`]), or the seed could not be used,
    /// for example because the password prompt was cancelled.
    pub(crate) async fn dashpay_open_contact_infos(
        &self,
        identity: &QualifiedIdentity,
        documents: &[Document],
    ) -> Result<PublishedContactInfos, TaskError> {
        if documents.is_empty() {
            return Ok(PublishedContactInfos::default());
        }
        let Some(seed_hash) = identity.dashpay_wallet_seed_hash() else {
            return Ok(PublishedContactInfos::default());
        };
        // A wallet that cannot be resolved is an error: a save keeps every
        // field it does not replace from this read, so an empty result here
        // would publish empty fields. Display callers degrade explicitly.
        let wallet = self.resolve_wallet(&seed_hash).await?;
        let owner = identity.identity.id();
        let identity_index = {
            let state = wallet.state().await;
            state
                .identity_manager
                .managed_identity(&owner)
                .and_then(|managed| managed.identity_index)
        };
        let Some(identity_index) = identity_index else {
            return Ok(PublishedContactInfos::default());
        };
        let network = identity.network;
        self.secret_access()
            .with_secret_session(&SecretScope::HdSeed { seed_hash }, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::ContactWalletSeedUnavailable)?;
                let crypto = ContactInfoCrypto::new(seed, network, identity_index);
                Ok(open_contact_info_documents(&crypto, documents).await)
            })
            .await
    }
}

/// The details published `contactInfo` documents hold, from the newest
/// document per contact.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PublishedContactInfos {
    details: BTreeMap<Identifier, ContactDetails>,
    /// Contacts whose newest document opens to their id but whose details do
    /// not decrypt or decode. It may also hold ids of no contact, from
    /// documents sealed under another key.
    unreadable: BTreeSet<Identifier>,
}

impl PublishedContactInfos {
    /// Take the published details for `contact_id`, and whether they are
    /// unreadable. Unreadable details stand in as hidden: their hidden flag
    /// cannot be read, and must not be taken as cleared.
    pub(crate) fn take(&mut self, contact_id: &Identifier) -> Option<(ContactDetails, bool)> {
        if self.unreadable.remove(contact_id) {
            let hidden = ContactDetails {
                hidden: true,
                ..Default::default()
            };
            return Some((hidden, true));
        }
        self.details
            .remove(contact_id)
            .map(|details| (details, false))
    }
}

/// Open `documents` (oldest first) through `crypto`; the newest document per
/// contact wins, readable or not.
// TODO(upstream): dip15-contactinfo-reader — replace with a platform-wallet
// contactInfo-only, provider-backed read for seedless wallets once one exists;
// platform-wallet's own drain is crate-private and runs unrelated contact work.
// Upstream request drafted, not yet filed.
async fn open_contact_info_documents(
    crypto: &ContactInfoCrypto<'_>,
    documents: &[Document],
) -> PublishedContactInfos {
    let mut opened = PublishedContactInfos::default();
    for document in documents {
        let Some(stored) = StoredContactInfo::of(document) else {
            continue;
        };
        let Ok(root_path) = identity_auth_derivation_path_for_type(
            crypto.network,
            KeyDerivationType::ECDSA,
            crypto.identity_index,
            stored.root_index,
        ) else {
            continue;
        };
        let (contact_id, data) = match crypto
            .contact_info_open(
                &root_path,
                stored.derivation_index,
                &stored.enc_to_user_id,
                stored.private_data,
            )
            .await
        {
            Ok(contact) => (
                contact.contact_id,
                decode_private_data(&contact.private_data).ok(),
            ),
            // The contact id still opens: the document is that contact's,
            // with details that do not decrypt.
            Err(_) => match crypto.keys(&root_path, stored.derivation_index) {
                Ok(keys) => (
                    platform_encryption::decrypt_enc_to_user_id(
                        &keys.enc_to_user_id_key,
                        &stored.enc_to_user_id,
                    ),
                    None,
                ),
                Err(_) => continue,
            },
        };
        let contact_id = Identifier::from(contact_id);
        match data {
            Some(data) => {
                opened.unreadable.remove(&contact_id);
                opened.details.insert(
                    contact_id,
                    ContactDetails {
                        nickname: data.alias_name,
                        note: data.note,
                        hidden: data.display_hidden,
                    },
                );
            }
            None => {
                opened.details.remove(&contact_id);
                opened.unreadable.insert(contact_id);
            }
        }
    }
    opened
}

/// The `contactInfo` fields a read needs.
struct StoredContactInfo<'a> {
    root_index: u32,
    derivation_index: u32,
    enc_to_user_id: [u8; 32],
    private_data: &'a [u8],
}

impl<'a> StoredContactInfo<'a> {
    fn of(document: &'a Document) -> Option<Self> {
        use dash_sdk::dpp::document::DocumentV0Getters;
        use dash_sdk::dpp::platform_value::Value;
        let properties = document.properties();
        let index = |name: &str| properties.get(name)?.to_integer::<u32>().ok();
        let bytes = |name: &str| match properties.get(name)? {
            Value::Bytes(bytes) => Some(bytes.as_slice()),
            Value::Bytes32(bytes) => Some(bytes.as_slice()),
            _ => None,
        };
        Some(Self {
            root_index: index("rootEncryptionKeyIndex")?,
            derivation_index: index("derivationEncryptionKeyIndex")?,
            enc_to_user_id: bytes("encToUserId")?.try_into().ok()?,
            private_data: bytes("privateData")?,
        })
    }
}

/// The wallet identity index a contact details write derives beneath,
/// checked before platform-wallet changes any local state.
fn contact_info_write_index(
    identity_index: Option<u32>,
    contact_is_established: bool,
) -> Result<u32, TaskError> {
    let identity_index = identity_index.ok_or(TaskError::ContactInfoIdentityNotFromWallet)?;
    if !contact_is_established {
        return Err(TaskError::ContactInfoContactNotEstablished);
    }
    Ok(identity_index)
}

/// Seals and opens `contactInfo` ciphertexts for one wallet identity.
///
/// Holds a borrow of the seed for one operation only; it never copies the
/// seed out of that operation.
pub(crate) struct ContactInfoCrypto<'a> {
    seed: &'a [u8; 64],
    network: Network,
    identity_index: u32,
}

impl<'a> ContactInfoCrypto<'a> {
    pub(crate) fn new(seed: &'a [u8; 64], network: Network, identity_index: u32) -> Self {
        Self {
            seed,
            network,
            identity_index,
        }
    }

    /// The two AES keys beneath `root_path`, derived by platform-wallet.
    ///
    /// `root_path` must be one of this identity's key paths: any other root is
    /// refused, so a request can never select another identity's keys.
    #[expect(
        clippy::result_large_err,
        reason = "ContactCryptoProvider fixes PlatformWalletError as the error type"
    )]
    fn keys(
        &self,
        root_path: &DerivationPath,
        derivation_index: u32,
    ) -> Result<ContactInfoKeys, PlatformWalletError> {
        let key_id = match root_path.len().checked_sub(1).map(|last| root_path[last]) {
            Some(ChildNumber::Hardened { index }) => index,
            _ => return Err(foreign_root()),
        };
        let own_root = identity_auth_derivation_path_for_type(
            self.network,
            KeyDerivationType::ECDSA,
            self.identity_index,
            key_id,
        )?;
        if own_root != *root_path {
            return Err(foreign_root());
        }
        let wallet =
            Wallet::from_seed_bytes(*self.seed, self.network, WalletAccountCreationOptions::None)
                .map_err(|error| PlatformWalletError::KeyDerivation(error.to_string()))?;
        derive_contact_info_keys(
            &wallet,
            self.network,
            self.identity_index,
            key_id,
            derivation_index,
        )
    }
}

fn foreign_root() -> PlatformWalletError {
    PlatformWalletError::InvalidParameter(
        "contactInfo root is not a key path of this identity".to_string(),
    )
}

/// The provider serves `contactInfo` only; contact requests, invitations and
/// auto-accept keys go through their own paths.
fn not_contact_info() -> PlatformWalletError {
    PlatformWalletError::InvalidParameter(
        "this crypto provider serves contactInfo only".to_string(),
    )
}

#[async_trait::async_trait]
impl ContactCryptoProvider for ContactInfoCrypto<'_> {
    async fn receiving_xpub(
        &self,
        _path: &DerivationPath,
    ) -> Result<ExtendedPubKey, PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn ecdh_shared_secret(
        &self,
        _path: &DerivationPath,
        _peer: &PublicKey,
    ) -> Result<Zeroizing<[u8; 32]>, PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn export_auto_accept_private_key(
        &self,
        _path: &DerivationPath,
    ) -> Result<SecretKey, PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn export_invitation_private_key(
        &self,
        _path: &DerivationPath,
    ) -> Result<SecretKey, PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn account_reference(
        &self,
        _path: &DerivationPath,
        _compact_xpub: &[u8],
        _account_index: u32,
        _version: u32,
    ) -> Result<u32, PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn unmask_account_reference(
        &self,
        _path: &DerivationPath,
        _compact_xpub: &[u8],
        _account_reference: u32,
    ) -> Result<(u32, u32), PlatformWalletError> {
        Err(not_contact_info())
    }

    async fn contact_info_seal(
        &self,
        root_path: &DerivationPath,
        derivation_index: u32,
        contact_id: &[u8; 32],
        private_data_plaintext: &[u8],
        private_data_iv: &[u8; 16],
    ) -> Result<ContactInfoSealed, PlatformWalletError> {
        let keys = self.keys(root_path, derivation_index)?;
        Ok(ContactInfoSealed {
            enc_to_user_id: platform_encryption::encrypt_enc_to_user_id(
                &keys.enc_to_user_id_key,
                contact_id,
            ),
            private_data: platform_encryption::encrypt_private_data(
                &keys.private_data_key,
                private_data_iv,
                private_data_plaintext,
            ),
        })
    }

    async fn contact_info_open(
        &self,
        root_path: &DerivationPath,
        derivation_index: u32,
        enc_to_user_id: &[u8; 32],
        private_data_blob: &[u8],
    ) -> Result<ContactInfoOpened, PlatformWalletError> {
        let keys = self.keys(root_path, derivation_index)?;
        let private_data =
            platform_encryption::decrypt_private_data(&keys.private_data_key, private_data_blob)
                .map_err(|error| {
                    PlatformWalletError::InvalidIdentityData(format!(
                        "contactInfo privateData could not be decrypted: {error}"
                    ))
                })?;
        Ok(ContactInfoOpened {
            contact_id: platform_encryption::decrypt_enc_to_user_id(
                &keys.enc_to_user_id_key,
                enc_to_user_id,
            ),
            private_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 64] = [0x42; 64];
    const NETWORK: Network = Network::Testnet;
    const IDENTITY_INDEX: u32 = 1;

    fn root(identity_index: u32, key_id: u32) -> DerivationPath {
        identity_auth_derivation_path_for_type(
            NETWORK,
            KeyDerivationType::ECDSA,
            identity_index,
            key_id,
        )
        .expect("root path")
    }

    fn upstream_keys(key_id: u32, derivation_index: u32) -> ContactInfoKeys {
        let wallet = Wallet::from_seed_bytes(SEED, NETWORK, WalletAccountCreationOptions::None)
            .expect("wallet");
        derive_contact_info_keys(&wallet, NETWORK, IDENTITY_INDEX, key_id, derivation_index)
            .expect("upstream keys")
    }

    /// What this provider seals, platform-wallet's own keys open: the
    /// documents are readable by every client that uses platform-wallet.
    #[tokio::test]
    async fn sealed_details_open_with_platform_wallet_keys() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let contact = [7u8; 32];
        let iv = [3u8; 16];

        let sealed = crypto
            .contact_info_seal(&root(IDENTITY_INDEX, 2), 5, &contact, b"note", &iv)
            .await
            .expect("seal");

        let keys = upstream_keys(2, 5);
        assert_eq!(
            platform_encryption::decrypt_enc_to_user_id(
                &keys.enc_to_user_id_key,
                &sealed.enc_to_user_id
            ),
            contact
        );
        assert_eq!(
            platform_encryption::decrypt_private_data(&keys.private_data_key, &sealed.private_data)
                .expect("decrypt"),
            b"note"
        );
    }

    /// A document platform-wallet sealed opens here.
    #[tokio::test]
    async fn opens_what_platform_wallet_sealed() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let keys = upstream_keys(2, 0);
        let contact = [9u8; 32];
        let enc = platform_encryption::encrypt_enc_to_user_id(&keys.enc_to_user_id_key, &contact);
        let blob =
            platform_encryption::encrypt_private_data(&keys.private_data_key, &[1; 16], b"alias");

        let opened = crypto
            .contact_info_open(&root(IDENTITY_INDEX, 2), 0, &enc, &blob)
            .await
            .expect("open");

        assert_eq!(opened.contact_id, contact);
        assert_eq!(opened.private_data, b"alias");
    }

    fn stored_document(
        id: u8,
        key_id: u32,
        derivation_index: u32,
        sealed: &ContactInfoSealed,
    ) -> Document {
        use dash_sdk::dpp::document::DocumentV0;
        use dash_sdk::dpp::platform_value::Value;
        Document::V0(DocumentV0 {
            id: Identifier::from([id; 32]),
            properties: BTreeMap::from([
                ("rootEncryptionKeyIndex".to_string(), Value::U32(key_id)),
                (
                    "derivationEncryptionKeyIndex".to_string(),
                    Value::U32(derivation_index),
                ),
                (
                    "encToUserId".to_string(),
                    Value::Bytes(sealed.enc_to_user_id.to_vec()),
                ),
                (
                    "privateData".to_string(),
                    Value::Bytes(sealed.private_data.clone()),
                ),
            ]),
            ..Default::default()
        })
    }

    async fn sealed(
        crypto: &ContactInfoCrypto<'_>,
        derivation_index: u32,
        contact: [u8; 32],
        details: ContactDetails,
    ) -> ContactInfoSealed {
        use platform_wallet::wallet::identity::crypto::{
            ContactInfoPrivateData, encode_private_data,
        };
        let plaintext = encode_private_data(&ContactInfoPrivateData {
            alias_name: details.nickname,
            note: details.note,
            display_hidden: details.hidden,
            accepted_accounts: Vec::new(),
        });
        crypto
            .contact_info_seal(
                &root(IDENTITY_INDEX, 2),
                derivation_index,
                &contact,
                &plaintext,
                &[4; 16],
            )
            .await
            .expect("seal")
    }

    /// SEC-003: DIP-0015 documents are read back without platform-wallet
    /// holding the seed; the newest per contact wins, and documents that do
    /// not open (another key root, or garbage) give no details and are not
    /// fatal.
    #[tokio::test]
    async fn dip15_documents_are_read_back_newest_first() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let note = |text: &str| ContactDetails {
            note: Some(text.into()),
            ..Default::default()
        };
        let older = sealed(&crypto, 0, [7; 32], note("older")).await;
        let newer = sealed(&crypto, 1, [7; 32], note("newer")).await;
        let hidden = sealed(
            &crypto,
            2,
            [8; 32],
            ContactDetails {
                hidden: true,
                ..Default::default()
            },
        )
        .await;
        let unreadable = ContactInfoSealed {
            enc_to_user_id: [1; 32],
            private_data: vec![9; 48],
        };
        let documents = [
            stored_document(1, 2, 0, &older),
            stored_document(2, 2, 9, &unreadable),
            stored_document(3, 2, 1, &newer),
            stored_document(4, 2, 2, &hidden),
        ];

        let opened = open_contact_info_documents(&crypto, &documents).await;

        assert_eq!(
            opened.details,
            BTreeMap::from([
                (Identifier::from([7; 32]), note("newer")),
                (
                    Identifier::from([8; 32]),
                    ContactDetails {
                        hidden: true,
                        ..Default::default()
                    }
                ),
            ])
        );
    }

    /// A newest document that names its contact but whose details do not
    /// decrypt or decode is not skipped in favour of an older readable one:
    /// the contact stays hidden, marked unreadable, since its hidden flag
    /// cannot be read.
    #[tokio::test]
    async fn an_unreadable_newest_document_keeps_its_contact_hidden() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let shown = ContactDetails {
            note: Some("older".into()),
            ..Default::default()
        };
        let older = sealed(&crypto, 0, [7; 32], shown.clone()).await;
        let mut undecryptable = sealed(&crypto, 1, [7; 32], shown.clone()).await;
        undecryptable.private_data = vec![9; 48];
        let older_other = sealed(&crypto, 2, [8; 32], shown.clone()).await;
        let undecodable = {
            let keys = crypto
                .keys(&root(IDENTITY_INDEX, 2), 3)
                .expect("own root keys");
            ContactInfoSealed {
                enc_to_user_id: platform_encryption::encrypt_enc_to_user_id(
                    &keys.enc_to_user_id_key,
                    &[8; 32],
                ),
                private_data: platform_encryption::encrypt_private_data(
                    &keys.private_data_key,
                    &[4; 16],
                    &[0xFF; 40],
                ),
            }
        };
        let documents = [
            stored_document(1, 2, 0, &older),
            stored_document(2, 2, 1, &undecryptable),
            stored_document(3, 2, 2, &older_other),
            stored_document(4, 2, 3, &undecodable),
        ];

        let mut opened = open_contact_info_documents(&crypto, &documents).await;

        let hidden = ContactDetails {
            hidden: true,
            ..Default::default()
        };
        for contact in [[7; 32], [8; 32]] {
            assert_eq!(
                opened.take(&Identifier::from(contact)),
                Some((hidden.clone(), true)),
                "{contact:?}"
            );
        }
    }

    /// A root that belongs to another wallet identity is refused.
    #[tokio::test]
    async fn another_identitys_root_is_refused() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let result = crypto
            .contact_info_seal(&root(IDENTITY_INDEX + 1, 2), 0, &[0; 32], b"", &[0; 16])
            .await;
        assert!(matches!(
            result,
            Err(PlatformWalletError::InvalidParameter(_))
        ));
    }

    /// A write is refused before platform-wallet runs when the identity has no
    /// wallet index or the contact is not established.
    #[test]
    fn a_write_needs_a_wallet_identity_and_an_established_contact() {
        assert!(matches!(
            contact_info_write_index(None, true),
            Err(TaskError::ContactInfoIdentityNotFromWallet)
        ));
        assert!(matches!(
            contact_info_write_index(Some(3), false),
            Err(TaskError::ContactInfoContactNotEstablished)
        ));
        assert_eq!(contact_info_write_index(Some(3), true).expect("index"), 3);
    }

    /// An identity whose wallet cannot be resolved is
    /// an error, not "no details published". A save reads this baseline for
    /// every field it keeps, so an empty one would publish empty fields.
    #[tokio::test]
    async fn an_unresolved_wallet_is_an_error_not_an_empty_baseline() {
        use crate::model::wallet::Wallet as DetWallet;
        use std::sync::RwLock;
        let dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let backend = context.wallet_backend().expect("backend wired");
        let wallet = DetWallet::new_from_seed(SEED, NETWORK, None, None).expect("wallet");
        let mut identity =
            crate::context::test_support::bare_user_identity(Identifier::from([5; 32]), NETWORK);
        identity.associated_wallets =
            BTreeMap::from([(wallet.seed_hash(), Arc::new(RwLock::new(wallet)))]);
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let sealed = crypto
            .contact_info_seal(&root(IDENTITY_INDEX, 2), 0, &[7; 32], b"", &[0; 16])
            .await
            .expect("seal");

        let result = backend
            .dashpay_open_contact_infos(&identity, &[stored_document(1, 2, 0, &sealed)])
            .await;

        assert!(
            matches!(result, Err(TaskError::WalletNotLoaded { .. })),
            "{result:?}"
        );
    }

    /// Everything other than contactInfo is refused rather than served.
    #[tokio::test]
    async fn other_crypto_requests_are_refused() {
        let crypto = ContactInfoCrypto::new(&SEED, NETWORK, IDENTITY_INDEX);
        let path = root(IDENTITY_INDEX, 0);
        assert!(crypto.receiving_xpub(&path).await.is_err());
        assert!(crypto.export_auto_accept_private_key(&path).await.is_err());
        assert!(crypto.export_invitation_private_key(&path).await.is_err());
        assert!(crypto.account_reference(&path, &[], 0, 0).await.is_err());
        assert!(
            crypto
                .unmask_account_reference(&path, &[], 0)
                .await
                .is_err()
        );
    }
}
