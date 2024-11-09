use crate::model::wallet::{Wallet, WalletSeedHash};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::enc::Encoder;
use bincode::error::{DecodeError, EncodeError};
use bincode::{BorrowDecode, Decode, Encode};
use dash_sdk::dashcore_rpc::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::bip32::ChildNumber;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::platform::IdentityPublicKey;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedIdentityPublicKey {
    pub identity_public_key: IdentityPublicKey,
    pub in_wallet_at_derivation_path: Option<(WalletSeedHash, DerivationPath)>,
}

impl Encode for QualifiedIdentityPublicKey {
    fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
        // Encode `identity_public_key`
        self.identity_public_key.encode(encoder)?;

        // Encode `in_wallet_at_derivation_path`
        match &self.in_wallet_at_derivation_path {
            Some((hash, derivation_path)) => {
                // Indicate that the option is `Some`
                true.encode(encoder)?;

                // Encode the `hash`
                hash.encode(encoder)?;

                // Encode the length of the `DerivationPath`
                derivation_path.len().encode(encoder)?;

                // Encode each `ChildNumber` in the `DerivationPath`
                for child in derivation_path.into_iter() {
                    match child {
                        ChildNumber::Normal { index } => {
                            0u8.encode(encoder)?; // Discriminant for Normal
                            index.encode(encoder)?;
                        }
                        ChildNumber::Hardened { index } => {
                            1u8.encode(encoder)?; // Discriminant for Hardened
                            index.encode(encoder)?;
                        }
                        ChildNumber::Normal256 { index } => {
                            2u8.encode(encoder)?; // Discriminant for Normal256
                            index.encode(encoder)?;
                        }
                        ChildNumber::Hardened256 { index } => {
                            3u8.encode(encoder)?; // Discriminant for Hardened256
                            index.encode(encoder)?;
                        }
                    }
                }
            }
            None => {
                // Indicate that the option is `None`
                false.encode(encoder)?;
            }
        }

        Ok(())
    }
}

impl Decode for QualifiedIdentityPublicKey {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, DecodeError> {
        // Decode `identity_public_key`
        let identity_public_key = IdentityPublicKey::decode(decoder)?;

        // Decode `in_wallet_at_derivation_path`
        let has_derivation_path = bool::decode(decoder)?;
        let in_wallet_at_derivation_path = if has_derivation_path {
            // Decode the `hash`
            let hash: [u8; 32] = Decode::decode(decoder)?;

            // Decode the length of the `DerivationPath`
            let path_len = usize::decode(decoder)?;

            // Decode each `ChildNumber` in the `DerivationPath`
            let mut path = Vec::with_capacity(path_len);
            for _ in 0..path_len {
                let discriminant = u8::decode(decoder)?;
                let child_number = match discriminant {
                    0 => ChildNumber::Normal {
                        index: u32::decode(decoder)?,
                    },
                    1 => ChildNumber::Hardened {
                        index: u32::decode(decoder)?,
                    },
                    2 => ChildNumber::Normal256 {
                        index: <[u8; 32]>::decode(decoder)?,
                    },
                    3 => ChildNumber::Hardened256 {
                        index: <[u8; 32]>::decode(decoder)?,
                    },
                    _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
                };
                path.push(child_number);
            }

            Some((hash, DerivationPath::from(path)))
        } else {
            None
        };

        Ok(Self {
            identity_public_key,
            in_wallet_at_derivation_path,
        })
    }
}

impl<'de> BorrowDecode<'de> for QualifiedIdentityPublicKey {
    fn borrow_decode<D: BorrowDecoder<'de>>(decoder: &mut D) -> Result<Self, DecodeError> {
        // Decode `identity_public_key`
        let identity_public_key = IdentityPublicKey::decode(decoder)?;

        // Decode `in_wallet_at_derivation_path`
        let has_derivation_path = bool::decode(decoder)?;
        let in_wallet_at_derivation_path = if has_derivation_path {
            // Decode the `hash`
            let hash: [u8; 32] = Decode::decode(decoder)?;

            // Decode the length of the `DerivationPath`
            let path_len = usize::decode(decoder)?;

            // Decode each `ChildNumber` in the `DerivationPath`
            let mut path = Vec::with_capacity(path_len);
            for _ in 0..path_len {
                let discriminant = u8::decode(decoder)?;
                let child_number = match discriminant {
                    0 => ChildNumber::Normal {
                        index: u32::decode(decoder)?,
                    },
                    1 => ChildNumber::Hardened {
                        index: u32::decode(decoder)?,
                    },
                    2 => ChildNumber::Normal256 {
                        index: <[u8; 32]>::decode(decoder)?,
                    },
                    3 => ChildNumber::Hardened256 {
                        index: <[u8; 32]>::decode(decoder)?,
                    },
                    _ => return Err(DecodeError::OtherString("Invalid ChildNumber type".into())),
                };
                path.push(child_number);
            }

            Some((hash, DerivationPath::from(path)))
        } else {
            None
        };

        Ok(Self {
            identity_public_key,
            in_wallet_at_derivation_path,
        })
    }
}

impl From<IdentityPublicKey> for QualifiedIdentityPublicKey {
    fn from(value: IdentityPublicKey) -> Self {
        Self {
            identity_public_key: value,
            in_wallet_at_derivation_path: None,
        }
    }
}

impl QualifiedIdentityPublicKey {
    pub fn from_identity_public_key_in_wallet(
        identity_public_key: IdentityPublicKey,
        in_wallet_at_derivation_path: Option<(WalletSeedHash, DerivationPath)>,
    ) -> Self {
        Self {
            identity_public_key,
            in_wallet_at_derivation_path,
        }
    }
    pub fn from_identity_public_key_with_wallets_check(
        value: IdentityPublicKey,
        network: Network,
        wallets: &[Arc<RwLock<Wallet>>],
    ) -> Self {
        // Initialize `in_wallet_at_derivation_path` as `None`
        let mut in_wallet_at_derivation_path = None;

        if let Ok(address) = value.address(network) {
            // Iterate over each wallet to check for matching derivation paths
            for locked_wallet in wallets {
                let wallet = locked_wallet.read().unwrap();
                if let Some(derivation_path) = wallet.known_addresses.get(&address) {
                    in_wallet_at_derivation_path =
                        Some((wallet.seed_hash(), derivation_path.clone()));
                }
                if in_wallet_at_derivation_path.is_some() {
                    break;
                }
            }
        }

        Self {
            identity_public_key: value,
            in_wallet_at_derivation_path,
        }
    }
}
