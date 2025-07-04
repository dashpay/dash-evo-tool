use dash_sdk::dpp::dashcore::bip32::DerivationPath;

/// Generic key handle used in the [GenericKms], used to identify keys in the KMS.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub enum GenericKeyHandle {
    PublicKeyBytes(Vec<u8>), // Public key bytes, used for encryption and verification
    Derived {
        seed_hash: Vec<u8>,              // Hash of the seed to use to derive the key
        derivation_path: DerivationPath, // Derivation path for the key; TODO:
    },
}
