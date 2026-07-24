mod derive_identity_key_for_display;
mod derive_key_for_display;
mod fetch_platform_address_balances;
mod fund_platform_address_from_asset_lock;
mod fund_platform_address_from_wallet_utxos;
mod generate_platform_receive_address;
mod generate_receive_address;
mod rename_wallet;
mod sign_message_with_identity_key;
mod sign_message_with_key;
mod transfer_platform_credits;
mod warm_identity_auth_pubkeys;
mod withdraw_from_platform_address;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};
use dash_sdk::dpp::dashcore::{OutPoint, PrivateKey};
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Build the Base64-encoded Dash signed-message envelope for `message` signed
/// with `secret_key`. The envelope is a recoverable signature: a header byte
/// (`27 + recId`, `+4` when `compressed`) followed by the 64-byte signature, so
/// a verifier can recover the signer's public key from the signature alone.
/// Shared by the wallet-key and identity-key message-signing tasks.
pub(crate) fn dash_signed_message(
    message: &str,
    secret_key: &SecretKey,
    compressed: bool,
) -> String {
    let secp = Secp256k1::new();
    let message_hash = signed_msg_hash(message);
    let digest = Message::from_digest(*message_hash.as_byte_array());
    let recoverable = secp.sign_ecdsa_recoverable(&digest, secret_key);
    MessageSignature::new(recoverable, compressed).to_base64()
}

impl AppContext {
    /// Resolve `seed_hash`'s wallet, derive its private key at
    /// `derivation_path` through the HD-seed JIT chokepoint, and hand it to
    /// `f`. The seed and derived key zeroize when the closure returns — only
    /// `f`'s result crosses back to the caller.
    ///
    /// `derivation_failed` is the `TaskError` reported when derivation itself
    /// fails; callers pass the variant matching their user-facing wording
    /// (message-signing vs. key-display differ). Shared by the wallet-key
    /// sign and display tasks.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::RootKeyDerivationRefused`] for an empty
    /// `derivation_path`: the empty path is the BIP-32 root, so deriving there
    /// would yield the wallet's master key instead of an address key. The
    /// refusal lives here, before the seed is fetched, because every
    /// key-bearing wallet task funnels through this seam — a UI-side gate would
    /// only cover the caller that happens to carry it.
    async fn with_wallet_derived_key<T>(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        derivation_path: &DerivationPath,
        derivation_failed: TaskError,
        f: impl FnOnce(PrivateKey) -> Result<T, TaskError>,
    ) -> Result<T, TaskError> {
        if derivation_path.as_ref().is_empty() {
            tracing::warn!("Refused a wallet-key derivation at the BIP-32 root path");
            return Err(TaskError::RootKeyDerivationRefused);
        }

        let wallet = self.wallet_arc(&seed_hash)?.read()?.clone();

        let network = self.network;
        let backend = self.wallet_backend()?;
        backend
            .secret_access()
            .with_secret(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                |plaintext| {
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let private_key = wallet
                        .private_key_at_derivation_path_with_seed(seed, derivation_path, network)
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Wallet-key derivation failed");
                            derivation_failed
                        })?;
                    f(private_key)
                },
            )
            .await
    }

    /// Resolve the vault-backed identity key at `(identity_id, target,
    /// key_id)` through the JIT chokepoint and hand its `SecretKey` to `f`.
    /// The raw key zeroizes when the closure returns — only `f`'s result
    /// crosses back to the caller. Shared by the identity-key sign and
    /// display tasks.
    async fn with_identity_secret_key<T>(
        self: &Arc<Self>,
        identity_id: Identifier,
        target: PrivateKeyTarget,
        key_id: KeyID,
        f: impl FnOnce(SecretKey) -> Result<T, TaskError>,
    ) -> Result<T, TaskError> {
        let scope = crate::wallet_backend::SecretScope::IdentityKey {
            identity_id: identity_id.to_buffer(),
            target,
            key_id,
        };
        let backend = self.wallet_backend()?;
        backend
            .secret_access()
            .with_secret(&scope, |plaintext| {
                let key = plaintext
                    .expose_identity_key()
                    .ok_or(TaskError::IdentityKeyMissing)?;
                // Present-but-malformed key bytes are distinct from a
                // genuinely absent key and from a signing/derivation failure.
                let secret_key = SecretKey::from_byte_array(key).map_err(|detail| {
                    tracing::warn!(error = %detail, "Identity-key secret construction failed");
                    TaskError::IdentityKeyMalformed
                })?;
                f(secret_key)
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WalletTask {
    GenerateReceiveAddress {
        seed_hash: WalletSeedHash,
    },
    /// Derive a private key for on-screen display/export. The HD seed is
    /// fetched just-in-time through the JIT chokepoint, the key is derived in
    /// the backend, and only the WIF (wrapped in `Secret`) is returned — the
    /// seed never crosses into the UI layer.
    DeriveKeyForDisplay {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
    },
    /// Generate a fresh Platform (DIP-17/18) receive address. The HD seed is
    /// fetched just-in-time through the JIT chokepoint, the address is derived
    /// and registered in the backend, and only the resulting address crosses
    /// back to the UI — the seed never leaves the backend.
    GeneratePlatformReceiveAddress {
        seed_hash: WalletSeedHash,
    },
    /// Warm the identity-authentication public-key cache for one identity
    /// index so the identity-key chooser can read its public keys without the
    /// seed. The HD seed is fetched just-in-time through the JIT chokepoint,
    /// the first `key_count` auth keys are derived and persisted to the cache
    /// in the backend, and only a completion signal crosses back to the UI —
    /// the seed never leaves the backend.
    WarmIdentityAuthPubkeys {
        seed_hash: WalletSeedHash,
        identity_index: u32,
        /// Number of auth keys to warm (master at index 0 plus the default
        /// additional keys), so the chooser's cache reads all hit.
        key_count: u32,
    },
    /// Sign a message with a wallet-derived key at `derivation_path`. The HD
    /// seed is fetched just-in-time through the JIT chokepoint, the key is
    /// derived and the message signed entirely in the backend, and only the
    /// Base64 signature (public) is returned — the seed and the derived private
    /// key never cross into the UI layer.
    SignMessageWithKey {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
        /// The message to sign (the user-entered plaintext, not a secret).
        message: String,
        /// The key type that determines the signing scheme.
        key_type: KeyType,
    },
    /// Derive an identity private key for on-screen display/export. The raw
    /// key is fetched just-in-time from the vault through the JIT chokepoint
    /// (`InVault` route) and only the WIF (wrapped in `Secret`) crosses back to
    /// the UI — the key bytes never become resident.
    DeriveIdentityKeyForDisplay {
        identity_id: Identifier,
        target: PrivateKeyTarget,
        key_id: KeyID,
    },
    /// Sign a message with a vault-backed identity key. The raw key is fetched
    /// just-in-time through the chokepoint, the message signed in the backend,
    /// and only the public Base64 signature crosses back — the key never
    /// becomes resident.
    SignMessageWithIdentityKey {
        identity_id: Identifier,
        target: PrivateKeyTarget,
        key_id: KeyID,
        /// The message to sign (the user-entered plaintext, not a secret).
        message: String,
        /// The key type that determines the signing scheme.
        key_type: KeyType,
    },
    /// Fetch Platform address balances and nonces from Platform for a wallet
    FetchPlatformAddressBalances {
        seed_hash: WalletSeedHash,
    },
    /// Transfer credits between Platform addresses
    TransferPlatformCredits {
        seed_hash: WalletSeedHash,
        /// Source addresses with amounts to transfer
        inputs: BTreeMap<PlatformAddress, Credits>,
        /// Destination addresses with amounts
        outputs: BTreeMap<PlatformAddress, Credits>,
        /// Index of the input to deduct fees from (in BTreeMap order).
        /// Should be the input with the highest balance to ensure sufficient funds for fees.
        fee_payer_index: u16,
    },
    /// List the wallet's tracked asset locks. Read through the upstream
    /// `AssetLockManager` (the single source of truth) off the UI thread, so
    /// screens never drive the async accessor from the egui frame loop.
    ListTrackedAssetLocks {
        seed_hash: WalletSeedHash,
    },
    /// Fund Platform addresses from a tracked asset lock identified by its
    /// credit-output outpoint. The proof and credit-output key are recovered
    /// from the upstream `AssetLockManager` and the wallet's funding
    /// account; DET no longer stages the asset lock itself.
    FundPlatformAddressFromAssetLock {
        seed_hash: WalletSeedHash,
        /// Credit-output outpoint of the tracked asset lock.
        out_point: OutPoint,
        /// Platform addresses and optional amounts to fund (None = distribute evenly)
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    },
    /// Withdraw from Platform addresses to Core
    WithdrawFromPlatformAddress {
        seed_hash: WalletSeedHash,
        /// Platform addresses and amounts to withdraw
        inputs: BTreeMap<PlatformAddress, Credits>,
        /// Core script to receive the withdrawal (e.g., P2PKH script)
        output_script: CoreScript,
        /// Core fee per byte
        core_fee_per_byte: u32,
        /// Index of the input to deduct fees from (in BTreeMap order).
        fee_payer_index: u16,
    },
    /// Fund a platform address directly from wallet UTXOs
    /// Creates asset lock, broadcasts, waits for proof, then funds platform address
    FundPlatformAddressFromWalletUtxos {
        seed_hash: WalletSeedHash,
        /// Amount in duffs to lock
        amount: u64,
        /// Destination platform address to fund
        destination: PlatformAddress,
        /// If true, fees are deducted from the output amount (recipient receives less).
        /// If false, fees are paid from extra wallet balance (recipient receives exact amount).
        fee_deduct_from_output: bool,
    },
    /// Persist a new alias for an HD wallet to the wallet-meta sidecar, off the
    /// UI thread. Reads the existing metadata fallibly so a storage/read failure
    /// surfaces instead of silently clobbering the other sidecar fields
    /// (`is_main` / `core_wallet_name` / xpub / password fields); a genuinely
    /// absent row is seeded fresh with the alias and the wallet's xpub. An empty
    /// `alias` clears the name.
    RenameHdWallet {
        seed_hash: WalletSeedHash,
        alias: String,
    },
    /// Persist a new alias for an imported single-key wallet to the single-key
    /// sidecar, off the UI thread. An empty `alias` clears the name.
    RenameSingleKeyWallet {
        address: String,
        alias: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::wallet::Wallet;
    use crate::model::wallet::birth_height::WalletOrigin;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::dashcore::secp256k1::PublicKey;
    use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};
    use dash_sdk::dpp::key_wallet::bip32::ChildNumber;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc::Receiver;

    /// An offline testnet context with one registered HD wallet whose seed sits
    /// in the vault. Key derivation is local, so no network is touched. The
    /// receiver and temp dir must outlive the context.
    struct WalletFixture {
        ctx: Arc<AppContext>,
        seed_hash: WalletSeedHash,
        _rx: Receiver<TaskResult>,
        _dir: tempfile::TempDir,
    }

    async fn wallet_fixture() -> WalletFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext");

        let (tx, rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());

        let seed = [0x5Au8; 64];
        let wallet =
            Wallet::new_from_seed(seed, Network::Testnet, None, None).expect("build wallet");
        let seed_hash = wallet.seed_hash();
        ctx.register_wallet(wallet, &seed, WalletOrigin::Fresh)
            .expect("register wallet");
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        WalletFixture {
            ctx,
            seed_hash,
            _rx: rx,
            _dir: dir,
        }
    }

    /// A BIP-44 external (receive) path `m/44'/1'/0'/0/index` on testnet.
    fn bip44_external(index: u32) -> DerivationPath {
        DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index },
        ])
    }

    /// An empty derivation path IS the BIP-32 root: deriving there yields the
    /// wallet's master key, not an address key. The chokepoint every key-bearing
    /// wallet task shares (display, message signing) must refuse it outright —
    /// the invariant cannot live in a UI button, which only guards one caller.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_derivation_path_is_refused_at_the_chokepoint() {
        let fixture = wallet_fixture().await;

        let reached_key = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&reached_key);
        let result = fixture
            .ctx
            .with_wallet_derived_key(
                fixture.seed_hash,
                &DerivationPath::from(Vec::new()),
                TaskError::WalletKeyLookupFailed,
                move |_master_key| {
                    probe.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
            .await;

        assert!(
            matches!(result, Err(TaskError::RootKeyDerivationRefused)),
            "deriving at the empty (root) path must be refused, not served"
        );
        assert!(
            !reached_key.load(Ordering::SeqCst),
            "the master key must never be derived, let alone handed to a caller"
        );
    }

    /// The root guard must not over-reach: a real address path still derives.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn known_derivation_path_still_derives() {
        let fixture = wallet_fixture().await;

        let wif = fixture
            .ctx
            .with_wallet_derived_key(
                fixture.seed_hash,
                &bip44_external(0),
                TaskError::WalletKeyLookupFailed,
                |key| Ok(key.to_wif()),
            )
            .await
            .expect("a known BIP-44 address path must still derive");

        assert!(!wif.is_empty(), "a derived key must produce a WIF");
    }

    /// The shared signed-message envelope round-trips: the signer's public key
    /// recovers from the produced signature for both compression flags. A
    /// hardcoded recovery header would fail ~50% of the time here. Both the
    /// wallet-key and identity-key signers call this one helper.
    fn assert_recovers(compressed: bool) {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_byte_array(&[0x42u8; 32]).expect("valid secret");
        let expected_pubkey = PublicKey::from_secret_key(&secp, &secret_key);
        let message = "Bilby was here";

        let base64 = dash_signed_message(message, &secret_key, compressed);
        let parsed = MessageSignature::from_base64(&base64).expect("valid envelope");
        assert_eq!(parsed.compressed, compressed);

        let recovered = parsed
            .recover_pubkey(&secp, signed_msg_hash(message))
            .expect("recovers a public key");
        assert_eq!(recovered.inner, expected_pubkey);
        assert_eq!(recovered.compressed, compressed);
    }

    #[test]
    fn recovers_signer_pubkey_compressed() {
        assert_recovers(true);
    }

    #[test]
    fn recovers_signer_pubkey_uncompressed() {
        assert_recovers(false);
    }
}
