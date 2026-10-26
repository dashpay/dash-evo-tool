//! A secp256k1 secret key that erases itself when dropped.

use dash_sdk::dpp::secp256k1;

/// A secp256k1 secret key erased when dropped.
///
/// `secp256k1::SecretKey` is `Copy` with no `Drop`, so every plain copy DET
/// builds from its self-zeroizing key material would otherwise outlive the
/// operation in freed memory. Wrapping the copy keeps the secp256k1 signing and
/// ECDH primitives (byte-identical output) while overwriting the key once the
/// operation finishes.
///
/// The erase is best effort: it uses secp256k1's volatile `non_secure_erase`,
/// and the compiler may still have moved the bytes elsewhere before. Not
/// `Clone`/`Copy` on purpose, so the only copy is the one that gets erased.
pub(crate) struct ErasedSecretKey(secp256k1::SecretKey);

impl ErasedSecretKey {
    /// Build a secret key from a 32-byte scalar, refusing zero and values not
    /// below the curve order.
    pub(crate) fn from_secret_bytes(scalar: &[u8; 32]) -> Result<Self, secp256k1::Error> {
        secp256k1::SecretKey::from_secret_bytes(*scalar).map(Self)
    }

    /// Overwrite the held key. Called on drop.
    fn erase(&mut self) {
        self.0.non_secure_erase();
    }
}

impl std::ops::Deref for ErasedSecretKey {
    type Target = secp256k1::SecretKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for ErasedSecretKey {
    fn drop(&mut self) {
        self.erase();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The erase that `Drop` runs leaves none of the original key bytes
    /// behind. Checked through `erase` itself, because reading a value after
    /// its drop has run is not allowed.
    #[test]
    fn erase_overwrites_the_key() {
        let scalar = [0x5a; 32];
        let mut key = ErasedSecretKey::from_secret_bytes(&scalar).expect("valid scalar");
        assert_eq!(key.to_secret_bytes(), scalar);

        key.erase();

        assert_ne!(key.to_secret_bytes(), scalar);
    }

    /// The wrapper refuses the same invalid scalars secp256k1 does.
    #[test]
    fn rejects_invalid_scalar() {
        assert!(ErasedSecretKey::from_secret_bytes(&[0; 32]).is_err());
        assert!(ErasedSecretKey::from_secret_bytes(&[0xff; 32]).is_err());
    }
}
