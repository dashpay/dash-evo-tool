use std::fmt;

pub type Hardened = bool;

// Define the IndexValue enum to represent either u64 or [u8; 32]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
enum IndexValue {
    U64(u64, Hardened),
    U256([u8; 32], Hardened),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct IndexConstPath<const N: usize> {
    indexes: [IndexValue; N],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct IndexPath {
    indexes: Vec<IndexValue>,
}

// Constants for feature purposes and sub-features
pub const FEATURE_PURPOSE: u64 = 9;
pub const DASH_COIN_TYPE: u64 = 5;
pub const DASH_TESTNET_COIN_TYPE: u64 = 1;
pub const FEATURE_PURPOSE_IDENTITIES: u64 = 5;
pub const FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION: u64 = 0;
pub const FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_REGISTRATION: u64 = 1;
pub const FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_TOPUP: u64 = 2;
pub const FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_INVITATIONS: u64 = 3;
pub const FEATURE_PURPOSE_DASHPAY: u64 = 15;
pub const IDENTITY_REGISTRATION_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_REGISTRATION, true),
    ],
};

pub const IDENTITY_REGISTRATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_REGISTRATION, true),
    ],
};

// Identity Top-Up Paths
pub const IDENTITY_TOPUP_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_TOPUP, true),
    ],
};

pub const IDENTITY_TOPUP_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_TOPUP, true),
    ],
};

// Identity Invitation Paths
pub const IDENTITY_INVITATION_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_INVITATIONS, true),
    ],
};

pub const IDENTITY_INVITATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_INVITATIONS, true),
    ],
};

// Authentication Keys Paths
pub const IDENTITY_AUTHENTICATION_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION, true),
    ],
};

pub const IDENTITY_AUTHENTICATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION, true),
    ],
};

impl<const N: usize> IndexConstPath<N> {
    // Create a new IndexPath with multiple indexes
    fn with_indexes(indexes: [IndexValue; N]) -> Self {
        IndexConstPath { indexes }
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
