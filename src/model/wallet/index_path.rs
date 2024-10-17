use std::fmt;
use std::slice::Iter;
use dash_sdk::dpp::dashcore::{PrivateKey, PublicKey, secp256k1};
use dash_sdk::dpp::dashcore::bip32::{ChildNumber, ExtendedPrivKey};
use dash_sdk::dpp::dashcore::key::Secp256k1;
use crate::model::wallet::derivation_path::{DerivationPathReference, DerivationPathType};

pub type Hardened = bool;

// Define the IndexValue enum to represent either u64 or [u8; 32]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum IndexValue {
    U64(u64, Hardened),
    U256([u8; 32], Hardened),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct IndexConstPath<const N: usize> {
    pub indexes: [IndexValue; N],
    pub reference: DerivationPathReference,
    pub path_type: DerivationPathType,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct IndexPath {
    pub indexes: Vec<IndexValue>,
}

pub trait Derive {
    fn indexes(&self) -> Iter<IndexValue>;

    fn derive_ecdsa_private<C: secp256k1::Signing>(
        &self,
        secp: &Secp256k1<C>,
        root_private_key: &ExtendedPrivKey,
    ) -> Result<ExtendedPrivKey, String> {
        let mut ext_priv_key = root_private_key.clone();

        for index_value in self.indexes() {
            match index_value {
                IndexValue::U64(index, hardened) => {
                    let child_number = if *hardened {
                        ChildNumber::from_hardened_idx(*index as u32)?
                    } else {
                        ChildNumber::from_normal_idx(*index as u32)?
                    };
                    ext_priv_key = ext_priv_key.ckd_priv(secp, child_number)?;
                }
                IndexValue::U256(_index_bytes, _hardened) => {
                    // Handle U256 indices appropriately
                    // For now, return an error
                    return Err(Bip32Error::CannotDeriveFromHardenedChild);
                }
            }
        }

        Ok(ext_priv_key)
    }
}

impl Derive for IndexPath {
    fn indexes(&self) -> Iter<IndexValue> {
        self.indexes.iter()
    }
}

impl<const N: usize> Derive for IndexConstPath<N> {
    fn indexes(&self) -> Iter<IndexValue> {
        self.indexes.iter()
    }
}

impl<const N: usize> IndexConstPath<N> {
    // Create a new IndexPath with multiple indexes
    fn with_indexes(indexes: [IndexValue; N], reference: DerivationPathReference, path_type: DerivationPathType) -> Self {
        IndexConstPath { indexes , reference, path_type }
    }
}

impl<const N: usize> fmt::Display for IndexConstPath<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indexes_str: Vec<String> = self
            .indexes
            .iter()
            .map(|index| match index {
                IndexValue::U64(u, hardened) => {
                    format!("U64({}{})", u, if *hardened { "'" } else { "" })
                }
                IndexValue::U256(arr, hardened) => format!(
                    "U256(0x{}{})",
                    hex::encode(arr),
                    if *hardened { "'" } else { "" }
                ),
            })
            .collect();
        write!(
            f,
            "UInt256IndexPath(length = {}): [{}]",
            self.indexes.len(),
            indexes_str.join(", ")
        )
    }
}

impl fmt::Display for IndexPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indexes_str: Vec<String> = self
            .indexes
            .iter()
            .map(|index| match index {
                IndexValue::U64(u, hardened) => {
                    format!("U64({}{})", u, if *hardened { "'" } else { "" })
                }
                IndexValue::U256(arr, hardened) => format!(
                    "U256(0x{}{})",
                    hex::encode(arr),
                    if *hardened { "'" } else { "" }
                ),
            })
            .collect();
        write!(
            f,
            "UInt256IndexPath(length = {}): [{}]",
            self.indexes.len(),
            indexes_str.join(", ")
        )
    }
}
