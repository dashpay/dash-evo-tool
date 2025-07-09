use dash_sdk::{
    dpp::{
        identity::{KeyType, identity_public_key::accessors::v0::IdentityPublicKeyGettersV0},
        platform_value::BinaryData,
    },
    platform::IdentityPublicKey,
};

use crate::kms::PublicKey;

pub trait SignatureVerifier<P> {
    fn verify(&self, public_key: &P, digest: &[u8]) -> Result<bool, String>;
}

pub struct Signature {
    pub signature: Vec<u8>,
    pub key_type: KeyType,
}
impl From<BinaryData> for Signature {
    fn from(value: BinaryData) -> Self {
        Self {
            signature: value.to_vec(),
            key_type: KeyType::ECDSA_SECP256K1, // Default or set based on context
        }
    }
}

impl From<Signature> for BinaryData {
    fn from(signature: Signature) -> Self {
        BinaryData::from(signature.signature)
    }
}

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        &self.signature
    }
}

impl AsRef<Vec<u8>> for Signature {
    fn as_ref(&self) -> &Vec<u8> {
        &self.signature
    }
}

impl Signature {
    pub fn new<D: AsRef<[u8]>, T: Into<KeyType>>(data: D, algorithm: T) -> Self {
        Self {
            signature: Vec::from(data.as_ref()),
            key_type: algorithm.into(),
        }
    }

    pub fn verify<P>(&self, public_key: P, digest: &[u8]) -> Result<(), String>
    where
        P: TryInto<PublicKey>,
    {
        let key: PublicKey = public_key
            .try_into()
            .map_err(|_| "Failed to convert public key".to_string())?;

        if self.key_type != key.key_type() {
            return Err(format!(
                "Key type mismatch: expected {:?}, got {:?}",
                self.key_type,
                key.key_type()
            ));
        }

        match self.key_type {
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                self.verify_ecdsa_signature(&key, digest)
            }
            KeyType::BLS12_381 => todo!("BLS signature verification not implemented yet"),
            KeyType::EDDSA_25519_HASH160 => {
                todo!("EdDSA signature verification not implemented yet")
            }
            _ => Err("Unsupported key type for signature verification".to_string()),
        }
    }

    fn verify_ecdsa_signature(&self, key: &PublicKey, data: &[u8]) -> Result<(), String> {
        dash_sdk::dpp::dashcore::signer::verify_data_signature(
            data,
            &self.signature,
            key.data().as_slice(),
        )
        .map_err(|e| e.to_string())
    }
}
