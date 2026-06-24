mod derive_identity_key_for_display;
mod derive_key_for_display;
mod fetch_platform_address_balances;
mod fund_platform_address_from_asset_lock;
mod fund_platform_address_from_wallet_utxos;
mod generate_platform_receive_address;
mod generate_receive_address;
mod sign_message_with_identity_key;
mod sign_message_with_key;
mod transfer_platform_credits;
mod warm_identity_auth_pubkeys;
mod withdraw_from_platform_address;

use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};
use dash_sdk::dpp::identity::core_script::CoreScript;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::platform::Identifier;
use std::collections::BTreeMap;

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
}

#[cfg(test)]
mod tests {
    use super::dash_signed_message;
    use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};

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
