use dash_sdk::dashcore_rpc::dashcore::bip32::{ChildNumber, ExtendedPubKey, KeyDerivationType};

use bitflags::bitflags;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{
    Address, InstantLock, Network, OutPoint, PrivateKey, PublicKey, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::{ContextProvider, Identity};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

use crate::kms::generic::key_handle::SeedHash;

pub mod dashcore;
pub mod platform;

/// Wallet trait for managing cryptocurrency wallets.
///
/// It provides the core and platform-specific functionality for handling
/// cryptocurrency transactions, including key management, signing, and
/// transaction creation.
pub trait DashWallet: ContextProvider {}

/// Single wallet, determined by a wallet seed, that can manage both Dash Core and Dash Platform
/// addresses and transactions.
pub struct Wallet {
    /// Dash Core wallet for managing Dash Core addresses and transactions
    pub dashcore: Arc<RwLock<dashcore::DashCoreWallet>>,
    /// Dash Platform wallet for managing Dash Platform identities and tokens
    pub platform: Arc<RwLock<platform::PlatformWallet>>,
}
