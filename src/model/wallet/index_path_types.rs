use crate::model::wallet::derivation_path::{DerivationPathReference, DerivationPathType};
use crate::model::wallet::index_path::{IndexConstPath, IndexValue};

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
    reference: DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

pub const IDENTITY_REGISTRATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_REGISTRATION, true),
    ],
    reference: DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

// Identity Top-Up Paths
pub const IDENTITY_TOPUP_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_TOPUP, true),
    ],
    reference: DerivationPathReference::BlockchainIdentityCreditTopupFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

pub const IDENTITY_TOPUP_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_TOPUP, true),
    ],
    reference: DerivationPathReference::BlockchainIdentityCreditTopupFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

// Identity Invitation Paths
pub const IDENTITY_INVITATION_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_INVITATIONS, true),
    ],
    reference: DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

pub const IDENTITY_INVITATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_INVITATIONS, true),
    ],
    reference: DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
    path_type: DerivationPathType::CREDIT_FUNDING,
};

// Authentication Keys Paths
pub const IDENTITY_AUTHENTICATION_PATH: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION, true),
    ],
    reference: DerivationPathReference::BlockchainIdentities,
    path_type: DerivationPathType::SINGLE_USER_AUTHENTICATION,
};

pub const IDENTITY_AUTHENTICATION_PATH_TESTNET: IndexConstPath<4> = IndexConstPath {
    indexes: [
        IndexValue::U64(FEATURE_PURPOSE, true),
        IndexValue::U64(DASH_TESTNET_COIN_TYPE, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES, true),
        IndexValue::U64(FEATURE_PURPOSE_IDENTITIES_SUBFEATURE_AUTHENTICATION, true),
    ],
    reference: DerivationPathReference::BlockchainIdentities,
    path_type: DerivationPathType::SINGLE_USER_AUTHENTICATION,
};