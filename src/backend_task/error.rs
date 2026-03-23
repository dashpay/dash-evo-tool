//! Typed error envelope for backend tasks.
//!
//! `Display` → user-friendly text (shown in `MessageBanner`).
//! `Debug` → variant name + fields (logged and shown in collapsible details).

use dash_sdk::Error as SdkError;
use dash_sdk::dashcore_rpc;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::basic::basic_error::BasicError;
use dash_sdk::dpp::consensus::state::state_error::StateError;
use dash_sdk::dpp::dashcore;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use thiserror::Error;

/// Dash Core RPC error code: wallet file not specified (multi-wallet node).
const RPC_WALLET_NOT_SPECIFIED: i32 = -19;

/// App-level error envelope for backend tasks.
#[derive(Debug, Error)]
pub enum TaskError {
    /// SPV subsystem errors.
    #[error("{}", spv_user_message(.0))]
    Spv(#[from] crate::spv::SpvError),

    /// DashPay domain errors.
    #[error(transparent)]
    DashPay(#[from] crate::backend_task::dashpay::errors::DashPayError),

    /// Configuration errors.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    /// GroveSTARK prover errors.
    #[error("Could not verify platform data. Please retry.")]
    GroveStark(#[from] crate::model::grovestark_prover::GroveSTARKError),

    /// Wallet errors.
    #[error(transparent)]
    Wallet(#[from] crate::database::WalletError),

    /// A local database operation failed.
    #[error("Could not access local data. Check available disk space and restart the application.")]
    Database {
        #[from]
        source: rusqlite::Error,
    },

    /// Tokio task join errors.
    #[error("An internal operation failed unexpectedly. Please restart the application.")]
    JoinError(#[from] tokio::task::JoinError),

    /// Core wallet not configured for this wallet on a multi-wallet Core node.
    #[error(
        "Core wallet not configured for this wallet. Go to the Wallets screen and refresh to auto-detect the Core wallet association."
    )]
    CoreWalletNotConfigured,

    /// Dash Core RPC rejected the request due to invalid credentials (HTTP 401).
    #[error("Dash Core rejected your credentials. Check your RPC password in settings.")]
    CoreRpcAuthFailed,

    /// Could not connect to Dash Core at the configured address.
    #[error(
        "Could not connect to Dash Core at {url}. Check that Dash Core is running and your network settings are correct."
    )]
    CoreRpcConnectionFailed {
        url: String,
        #[source]
        source: dashcore_rpc::Error,
    },

    /// A Dash Core RPC call failed.
    #[error("Could not communicate with Dash Core. Check that Dash Core is running and retry.")]
    CoreRpc {
        #[source]
        source: dashcore_rpc::Error,
    },

    /// An internal lock was poisoned — another thread panicked while holding it.
    #[error("An internal error occurred. Please restart the application.")]
    LockPoisoned {
        /// Which resource's lock was poisoned (for Debug / logs).
        resource: &'static str,
    },

    /// The requested wallet was not found in the local wallet store.
    #[error("Wallet not found. Please check your wallet list and try again.")]
    WalletNotFound,

    /// The wallet is locked and must be unlocked before this operation can proceed.
    #[error("Wallet is locked. Please unlock your wallet and try again.")]
    WalletLocked,

    /// The requested document could not be found on the platform.
    #[error("The document could not be found. It may have been deleted or the ID is incorrect.")]
    DocumentNotFound,

    /// An asset lock's instant-lock proof has expired before Platform verified it.
    #[error(
        "This transaction cannot be used yet because its verification has expired. \
         The network is still processing earlier blocks. \
         Please wait a few minutes and retry."
    )]
    AssetLockExpired {
        tx_block_height: u32,
        platform_height: u32,
    },

    /// The private key for the asset lock address was not found in the wallet.
    #[error(
        "The address for this transaction could not be found in your wallet. \
         Make sure you are using the correct wallet."
    )]
    AssetLockAddressNotFound,

    /// A state transition was broadcast but proof verification failed; the proof has been logged.
    #[error(
        "The operation could not be fully verified by the platform. The issue has been logged. \
         Please check whether the operation completed and retry if needed."
    )]
    ProofError {
        /// The original SDK error that triggered proof-verification failure.
        #[source]
        source_error: Box<SdkError>,
    },

    /// The requested identity was not found on the platform.
    #[error("Identity not found on the platform. Please check the ID or name and try again.")]
    IdentityNotFound,

    /// Timed out waiting for transaction confirmation.
    #[error(
        "The transaction was not confirmed within the expected time. Please check your network connection and retry."
    )]
    ConfirmationTimeout,

    /// Dash Core peer-to-peer communication failed.
    #[error(transparent)]
    P2P(#[from] crate::components::core_p2p_handler::P2PError),

    /// The operation's prerequisite was auto-fixed (e.g., Core wallet detected).
    /// Callers should retry the failed operation.
    #[error("{0}")]
    MustRetry(String),

    /// Duplicate identity public key — the key data already exists on the platform.
    #[error("This public key is already registered on the platform. Try a different key.")]
    DuplicateIdentityPublicKey {
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// Duplicate identity public key ID — the key hash is already taken platform-wide.
    #[error("This key is already registered on the platform. Try a different key.")]
    DuplicateIdentityPublicKeyId {
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// Identity public key conflicts with an existing key's unique contract bounds.
    #[error(
        "This key conflicts with an existing key bound to contract {contract_id}. Use a different key or purpose."
    )]
    IdentityPublicKeyContractBoundsConflict {
        contract_id: String,
        /// The original SDK error returned by the broadcast API.
        #[source]
        source_error: Box<SdkError>,
    },

    /// The identity could not be found in the local wallet database.
    #[error(
        "This identity could not be found in your local wallet. Try refreshing your identities list."
    )]
    IdentityNotFoundLocally,

    /// Failed to build the identity update state transition.
    #[error("Could not build the key update transaction. Please retry.")]
    IdentityUpdateTransitionError {
        #[source]
        source_error: Box<SdkError>,
    },

    /// Failed to send a result back to the UI — the receiver was dropped.
    #[error("Internal update failed. Please retry the operation.")]
    InternalSendError,

    /// Unclassified SDK error — the operation failed for an unrecognised reason.
    /// Display is implemented manually via [`sdk_error_user_message`] to inspect
    /// the source error and produce an actionable, user-friendly message.
    #[error("{}", sdk_error_user_message(source_error))]
    SdkError {
        #[source]
        source_error: Box<SdkError>,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Wallet / platform-address operation errors
    // ──────────────────────────────────────────────────────────────────────────
    /// Wallet address provider could not be set up (wallet is open but derivation failed).
    #[error(
        "Could not prepare wallet addresses for sync. Please close and reopen your wallet, then retry."
    )]
    WalletAddressProviderSetupFailed { detail: String },

    /// A Core address could not be converted to a Platform address.
    #[error("Could not convert a wallet address for platform use. Please retry.")]
    AddressConversionFailed {
        #[source]
        source: Box<ProtocolError>,
    },

    /// Overflow while converting duffs to platform credits.
    #[error("The amount is too large to process. Please use a smaller amount.")]
    CreditCalculationOverflow { amount: u64, credits_per_duff: u64 },

    /// A change address could not be derived or located in the outputs map.
    #[error("Could not prepare a change address for this transaction. Please retry.")]
    ChangeAddressUnavailable { reason: &'static str },

    // ──────────────────────────────────────────────────────────────────────────
    // Asset-lock transaction errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The asset lock transaction was expected in the local database but was not found.
    #[error(
        "The funding transaction could not be found locally. Please check your network connection and retry."
    )]
    AssetLockTransactionNotFoundInDatabase,

    /// An asset lock transaction has no credit outputs (malformed transaction).
    #[error(
        "The funding transaction is missing required outputs and cannot be used. Please retry creating the transaction."
    )]
    AssetLockNoCreditOutputs,

    /// Could not derive a Core address from an asset lock output script.
    #[error("Could not read the address from the funding transaction. Please retry.")]
    AssetLockAddressDerivationFailed {
        #[source]
        source: dashcore::address::Error,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Token contract errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A token at the expected position was not found in the contract.
    #[error(
        "Token at position {position} was not found in the contract. Please reload the contract and retry."
    )]
    TokenPositionNotFound { position: u16 },

    // ──────────────────────────────────────────────────────────────────────────
    // Contract errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The requested data contract could not be found locally or on the platform.
    #[error(
        "The data contract could not be found. It may have been removed or the ID is incorrect."
    )]
    DataContractNotFound,

    // ──────────────────────────────────────────────────────────────────────────
    // Serialization errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A data serialization or deserialization operation failed (e.g. bincode).
    #[error("Could not process the data. Please retry the operation.")]
    SerializationError { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Identity creation / parsing errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The provided identifier could not be parsed from the input.
    #[error("The identifier you entered could not be read. Please check the format and try again.")]
    IdentifierParsingError { input: String },

    /// The identity could not be constructed from the given parameters.
    #[error("Could not create the identity. Please check your input and try again.")]
    IdentityCreationError {
        #[source]
        source: Box<ProtocolError>,
    },

    /// A private key could not be parsed or is invalid.
    #[error("The private key you entered is invalid. Please check the format and try again.")]
    InvalidPrivateKey { detail: String },

    /// Fetching DPNS names for an identity failed.
    #[error("Could not look up names for this identity. Please check your connection and retry.")]
    DpnsFetchError {
        #[source]
        source: Box<SdkError>,
    },

    /// An asset lock's private key could not be matched to a wallet address.
    #[error(
        "The funding transaction does not match your wallet. \
         Make sure you are using the correct wallet."
    )]
    AssetLockNotValidForWallet,

    /// The instant lock proof has expired and the transaction is not yet chain-locked.
    #[error(
        "This funding transaction cannot be used right now. The verification has expired and the \
         transaction is not yet confirmed. Please wait a few minutes and retry."
    )]
    AssetLockInstantLockExpiredNotChainlocked,

    /// The instant lock proof signature could not be verified by the platform.
    #[error(
        "The transaction could not be verified instantly. \
         Please wait for it to be included in a block and retry."
    )]
    AssetLockInstantLockProofInvalid {
        #[source]
        source_error: Box<SdkError>,
    },

    /// The identity doesn't have enough Platform credits for this operation.
    #[error(
        "Not enough Platform credits. Your identity has {available} credits \
         but this operation requires {required}. Please top up your identity first."
    )]
    IdentityInsufficientBalance {
        available: u64,
        required: u64,
        #[source]
        source_error: Box<SdkError>,
    },

    /// Fetching address information from the platform failed.
    #[error("Could not retrieve address information from the platform. Please retry.")]
    PlatformFetchError {
        #[source]
        source: Box<SdkError>,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Dash Core lifecycle errors
    // ──────────────────────────────────────────────────────────────────────────
    /// Dash Core could not be started (binary missing, config error, I/O failure).
    #[error("Could not start Dash Core. Verify the installation and try again.")]
    DashCoreStartError {
        #[source]
        source: std::io::Error,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Network restriction errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The requested operation is not available on the current network.
    #[error(
        "{operation} is only available on {allowed_networks}. Switch to a supported network and retry."
    )]
    OperationNotAvailableOnNetwork {
        operation: &'static str,
        allowed_networks: &'static str,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Platform info errors
    // ──────────────────────────────────────────────────────────────────────────
    /// Fetching platform information failed.
    #[error("Could not retrieve platform information. Please check your connection and retry.")]
    PlatformInfoFetchError {
        #[source]
        source: Box<SdkError>,
    },

    // ──────────────────────────────────────────────────────────────────────────
    // Encryption errors
    // ──────────────────────────────────────────────────────────────────────────
    /// An encryption or decryption operation failed.
    #[error("Could not process encrypted data. Please check your keys and try again.")]
    EncryptionError { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Wallet persistence errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A wallet record could not be found or updated in the local database.
    #[error("Could not update wallet settings. Please restart the application and try again.")]
    WalletDatabasePersistError,

    // ──────────────────────────────────────────────────────────────────────────
    // Identity key errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The identity's master key was not found in the local key store.
    #[error(
        "The master key for this identity could not be found. Make sure the identity was created from this wallet."
    )]
    MasterKeyNotFound,

    // ──────────────────────────────────────────────────────────────────────────
    // Token query errors
    // ──────────────────────────────────────────────────────────────────────────
    /// Querying token data from the platform failed.
    #[error("Could not retrieve token information from the platform. Please retry.")]
    TokenQueryError { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Contract schema errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The contract structure does not match expectations (e.g. missing contested index).
    #[error("The contract structure is unexpected. Please update the application.")]
    ContractSchemaMismatch { detail: &'static str },

    // ──────────────────────────────────────────────────────────────────────────
    // Withdrawal document parsing errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A withdrawal document could not be fully read (missing timestamp, invalid status, etc.).
    #[error(
        "Could not read the withdrawal details. The data may be incomplete or in an unexpected format. Please retry."
    )]
    WithdrawalDocumentParsingError { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // SDK / RPC setup errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The Dash Platform SDK could not be initialised with the current config,
    /// or a context provider could not be bound to the current AppContext.
    #[error(
        "Could not connect to the Dash network. Please check your network settings and restart the application."
    )]
    SdkInitializationFailed { detail: String },

    /// An RPC context provider or Core RPC client could not be constructed.
    #[error("Could not set up the Dash Core connection. Please check your settings and retry.")]
    RpcProviderCreationFailed { detail: String },

    /// The Core wallet name supplied by the user is syntactically invalid.
    #[error("The Core wallet name '{name}' is invalid. Please check your wallet configuration.")]
    InvalidCoreWalletName { name: String },

    /// Dash Core has no wallets loaded — required for wallet-scoped RPC calls.
    #[error("No wallets are loaded in Dash Core. Please open a wallet in Dash Core and retry.")]
    NoCoreWalletsLoaded,

    // ──────────────────────────────────────────────────────────────────────────
    // SPV operation errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The SPV data directory could not be cleared.
    #[error(
        "Could not clear SPV data. Please close the application and manually delete the SPV data directory."
    )]
    SpvClearDataFailed { detail: String },

    /// The SPV client could not be started.
    #[error("Could not start the SPV client. Please check your network settings and retry.")]
    SpvStartFailed { detail: String },

    /// A transaction could not be broadcast via the SPV client.
    #[error("Could not broadcast the transaction. Please check your connection and retry.")]
    SpvBroadcastFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // UTXO / asset-lock transaction build errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A UTXO reload or removal operation failed.
    #[error(
        "Could not update your unspent transaction outputs. Please check your connection and retry."
    )]
    UtxoUpdateFailed { detail: String },

    /// An asset lock transaction could not be built from the current wallet state.
    #[error(
        "Could not prepare the funding transaction. Please check your wallet balance and retry."
    )]
    AssetLockTransactionBuildFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Wallet key / address errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A private key for a wallet address could not be found.
    #[error(
        "Could not find the key for this address in your wallet. Please check your wallet and retry."
    )]
    WalletKeyLookupFailed { detail: String },

    /// A new receive or change address could not be derived from the wallet.
    #[error("Could not generate a wallet address. Please check your wallet and retry.")]
    WalletAddressDerivationFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Payment errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A recipient address could not be parsed or is invalid.
    #[error("The recipient address '{address}' is not valid. Please check the address and retry.")]
    InvalidRecipientAddress {
        address: String,
        #[source]
        source: dashcore::address::Error,
    },

    /// A recipient address was parsed but does not match the current network.
    #[error(
        "The address does not match the current network. Please check that you are on the correct network."
    )]
    AddressNetworkMismatch {
        #[source]
        source: dashcore::address::Error,
    },

    /// The wallet has no UTXOs available to cover the payment.
    #[error("Your wallet has no available funds to spend. Please receive some Dash first.")]
    NoUtxosAvailable,

    /// The wallet balance is too low to cover the requested amount plus fees.
    #[error(
        "You do not have enough Dash. You have {available} duffs but need {required} duffs. Please add more funds and retry."
    )]
    InsufficientFunds { available: u64, required: u64 },

    /// The output amount is smaller than the transaction fee.
    #[error(
        "The amount is too small to cover the {fee} duff transaction fee. Please send a larger amount."
    )]
    OutputTooSmallForFee { fee: u64 },

    /// A signature hash for a transaction input could not be computed.
    #[error("Could not sign the transaction. Please retry.")]
    SighashComputationFailed {
        #[source]
        source: dashcore::sighash::Error,
    },

    /// A wallet payment operation failed (covers SPV and RPC payment paths).
    #[error("Could not complete the payment. Please check your wallet balance and retry.")]
    WalletPaymentFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Token query errors (identity / recipient validation)
    // ──────────────────────────────────────────────────────────────────────────
    /// No local identities are registered — a prerequisite for token queries.
    #[error("No registered identities found. Please register an identity first.")]
    NoIdentitiesFound,

    /// The current identity is not the contract owner who can claim this token's distribution.
    #[error(
        "This token distribution can only be claimed by the contract owner ({contract_owner}). Your identity is not the contract owner."
    )]
    NotContractOwner { contract_owner: String },

    /// The current identity is not the specific identity designated as the token distribution recipient.
    #[error(
        "This token distribution can only be claimed by the designated recipient ({designated_recipient}). Your identity is not the designated recipient."
    )]
    NotDesignatedTokenRecipient { designated_recipient: String },

    /// The current identity is not an evonode, which is required for this token distribution.
    #[error(
        "This token distribution is only for evonode identities. Your identity is not registered as an evonode."
    )]
    NotEvonode,

    // ──────────────────────────────────────────────────────────────────────────
    // Wallet-based identity loading errors
    // ──────────────────────────────────────────────────────────────────────────
    /// No on-chain identity was found for the requested wallet derivation index.
    #[error(
        "Could not find an identity for wallet index {identity_index} after checking {auth_key_count} keys. Try expanding the search range."
    )]
    WalletIdentityNotFound {
        identity_index: u32,
        auth_key_count: usize,
    },

    /// The identity returned by the platform does not contain the queried authentication key.
    #[error(
        "The identity retrieved does not match your wallet key. Please check you are using the correct wallet."
    )]
    WalletIdentityKeyMismatch,

    /// None of the identity's public keys could be matched to wallet derivation paths.
    #[error(
        "Could not match any identity keys to your wallet. Please check your wallet and retry."
    )]
    NoMatchingWalletKeys,

    /// The derivation path for the queried identity key was not found in the wallet.
    #[error(
        "Could not locate this identity key's information in your wallet. Please check your wallet configuration."
    )]
    WalletKeyDerivationPathNotFound,

    /// Wallet scan completed but no identities were found up to the requested index.
    #[error("No identities found up to wallet index {max_index}. Try a higher search range.")]
    NoWalletIdentitiesFound { max_index: u32 },

    // ──────────────────────────────────────────────────────────────────────────
    // Key input validation errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A raw private-key input string failed format validation.
    #[error("The {key_name} key is invalid: {detail}. Please check the key format and retry.")]
    KeyInputValidationFailed { key_name: String, detail: String },

    /// The identity's public keys could not be converted to the platform format.
    #[error("Could not process the identity keys. Please check your key configuration and retry.")]
    PublicKeyMapBuildFailed { detail: String },

    /// The wallet-binding information for an identity could not be determined.
    #[error(
        "Could not read wallet information for this identity. Please check your wallet and retry."
    )]
    WalletInfoDeterminationFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Voting / DPNS errors
    // ──────────────────────────────────────────────────────────────────────────
    /// A qualified identity does not have an associated voter identity.
    #[error(
        "The identity {identity_id} does not have a voting key. Please add a voting key to vote."
    )]
    NoVotingIdentity { identity_id: String },

    /// The identity does not have an authentication key required to sign documents.
    #[error(
        "This identity does not have a key for signing documents. Please add an authentication key."
    )]
    NoDocumentSigningKey,

    // ──────────────────────────────────────────────────────────────────────────
    // Wallet creation / import errors
    // ──────────────────────────────────────────────────────────────────────────
    /// The wallet has already been imported for this network.
    #[error("This wallet has already been imported for this network.")]
    WalletAlreadyImported,

    /// Wallet key derivation failed during construction.
    #[error("Could not create the wallet. Key derivation failed — please try again.")]
    WalletKeyDerivationFailed { detail: String },

    // ──────────────────────────────────────────────────────────────────────────
    // Shielded pool errors
    // ──────────────────────────────────────────────────────────────────────────
    /// No unspent shielded notes are available.
    #[error("You have no shielded funds available. Please shield some credits first.")]
    ShieldedNoUnspentNotes,

    /// Insufficient shielded balance to cover the requested amount.
    #[error(
        "Insufficient shielded balance: you have {available} credits but need {required}. Please shield more credits."
    )]
    ShieldedInsufficientBalance { available: u64, required: u64 },

    /// The platform address was not found in the wallet's platform address info.
    #[error("The platform address could not be found in your wallet. Please refresh and retry.")]
    PlatformAddressNotFound,

    /// A Merkle witness could not be obtained for a shielded note.
    #[error("Could not prepare the shielded transaction. Please sync your notes and retry.")]
    ShieldedMerkleWitnessUnavailable { detail: String },

    /// Failed to build a shielded state transition (shield, transfer, unshield, withdrawal).
    #[error("Could not build the shielded transaction. Please retry.")]
    ShieldedTransitionBuildFailed { detail: String },

    /// Failed to broadcast a shielded state transition.
    #[error(
        "Could not broadcast the shielded transaction. Please check your connection and retry."
    )]
    ShieldedBroadcastFailed {
        #[source]
        source: Box<dash_sdk::Error>,
    },

    /// Invalid recipient address for shielded transfer.
    #[error("The recipient shielded address is invalid. Please check the address and retry.")]
    ShieldedInvalidRecipientAddress,

    /// Timed out waiting for asset lock proof during shield-from-asset-lock.
    #[error(
        "The funding transaction was not confirmed within 5 minutes. Please check your network connection and retry."
    )]
    ShieldedAssetLockTimeout,

    /// Failed to sync shielded notes from the platform.
    #[error(
        "Could not sync shielded notes from the platform. Please check your connection and retry."
    )]
    ShieldedSyncFailed { detail: String },

    /// Failed to append or checkpoint the shielded commitment tree.
    #[error(
        "Could not update the local shielded data. Please check available disk space and retry."
    )]
    ShieldedTreeUpdateFailed { detail: String },

    /// Nullifier sync failed.
    #[error("Could not check for spent shielded notes. Please check your connection and retry.")]
    ShieldedNullifierSyncFailed { detail: String },
}

/// Returns `true` when a `dashcore_rpc::Error` wraps an HTTP 401 response,
/// indicating the RPC server rejected the supplied credentials.
pub fn is_rpc_auth_error(e: &dashcore_rpc::Error) -> bool {
    if let dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Transport(boxed)) = e
        && let Some(dashcore_rpc::jsonrpc::simple_http::Error::HttpErrorCode(401)) =
            boxed.downcast_ref::<dashcore_rpc::jsonrpc::simple_http::Error>()
    {
        return true;
    }
    false
}

/// Returns `true` when the RPC error indicates a transport-level connection
/// failure (refused, reset, timeout) as opposed to a protocol-level error.
/// Excludes HTTP status code errors (like 401) which are auth, not connection.
pub fn is_rpc_connection_error(e: &dashcore_rpc::Error) -> bool {
    if let dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Transport(boxed)) = e
        && let Some(http_err) = boxed.downcast_ref::<dashcore_rpc::jsonrpc::simple_http::Error>()
    {
        return matches!(
            http_err,
            dashcore_rpc::jsonrpc::simple_http::Error::SocketError(_)
        );
    }
    false
}

/// Returns `true` when the SDK error indicates an invalid instant asset lock
/// proof signature — the structured equivalent of the old string-matching
/// on `"Instant lock proof signature is invalid"`.
pub fn is_instant_lock_proof_invalid(error: &SdkError) -> bool {
    let consensus_error = match error {
        SdkError::StateTransitionBroadcastError(broadcast_err) => broadcast_err.cause.as_ref(),
        SdkError::Protocol(ProtocolError::ConsensusError(ce)) => Some(ce.as_ref()),
        _ => None,
    };
    matches!(
        consensus_error,
        Some(ConsensusError::BasicError(
            BasicError::InvalidInstantAssetLockProofSignatureError(_),
        ))
    )
}

/// Produce a user-friendly message for SPV subsystem errors.
///
/// Inspects the specific `SpvError` variant to give actionable guidance.
fn spv_user_message(e: &crate::spv::SpvError) -> &'static str {
    use crate::spv::SpvError;
    match e {
        SpvError::LockPoisoned(_) | SpvError::ChannelError(_) => {
            "An internal error occurred. Please restart the application."
        }
        SpvError::ClientNotInitialized | SpvError::NotRunning => {
            "The wallet sync service is not ready. Please restart the application."
        }
        SpvError::NetworkError(_) | SpvError::SyncFailed(_) => {
            "Could not sync wallet data. Please check your connection and retry."
        }
        SpvError::WalletError(_) => {
            "Could not process wallet data. Please check your wallet and retry."
        }
        SpvError::ConfigError(_) => {
            "Wallet sync is not configured properly. Please check your settings."
        }
        SpvError::Other(_) => "Could not sync wallet data. Please retry.",
    }
}

/// Produce a user-friendly message by inspecting the SDK error variant.
///
/// The returned text is shown in `MessageBanner` via `Display`.
/// Technical details remain available through the `#[source]` chain / `Debug`.
///
/// TODO: Expand match arms as we encounter more SDK error variants in the wild.
/// Each arm should explain *what happened* and *what the user can do*.
fn sdk_error_user_message(error: &SdkError) -> String {
    match error {
        SdkError::StateTransitionBroadcastError(_) => {
            "The platform rejected this request. Please check your input and try again."
                .to_string()
        }
        SdkError::TimeoutReached(duration, _) => {
            format!(
                "The operation did not complete within {} seconds. Please retry — it often succeeds on the second attempt.",
                duration.as_secs()
            )
        }
        SdkError::StaleNode(_) => {
            "The server you connected to is behind. Please retry — the app will pick a different server automatically.".to_string()
        }
        SdkError::DapiClientError(_) => {
            "Could not connect to the Dash network. Please retry in a few moments.".to_string()
        }
        SdkError::NoAvailableAddressesToRetry(_) => {
            "All Dash network servers are temporarily unreachable. Please wait a minute and retry.".to_string()
        }
        SdkError::Cancelled(_) => "The operation was cancelled.".to_string(),
        SdkError::AlreadyExists(_) => {
            "This object already exists on the platform.".to_string()
        }
        SdkError::NonceOverflow(_) => {
            "This identity has reached its maximum number of operations. Please try again later.".to_string()
        }
        SdkError::IdentityNonceNotFound(_) => {
            "The platform has not indexed this identity yet. Please retry in a few moments.".to_string()
        }
        _ => {
            "An unexpected error occurred. Please try again later.".to_string()
        }
    }
}

/// Blanket conversion for lock poisoning errors. This is the recommended approach:
/// use `?` on `.read()`, `.write()`, or `.lock()` calls instead of explicit `map_err`.
/// The resource name is derived from `type_name::<T>()` automatically.
impl<T> From<std::sync::PoisonError<T>> for TaskError {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        TaskError::LockPoisoned {
            resource: std::any::type_name::<T>(),
        }
    }
}

impl From<dashcore_rpc::Error> for TaskError {
    fn from(e: dashcore_rpc::Error) -> Self {
        if is_rpc_auth_error(&e) {
            return TaskError::CoreRpcAuthFailed;
        }
        if let dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(ref rpc_err)) =
            e
            && rpc_err.code == RPC_WALLET_NOT_SPECIFIED
        {
            return TaskError::CoreWalletNotConfigured;
        }
        TaskError::CoreRpc { source: e }
    }
}

impl From<SdkError> for TaskError {
    fn from(error: SdkError) -> Self {
        enum ConsensusKind {
            DuplicateKey,
            DuplicateKeyId,
            ContractBoundsConflict(String),
            InvalidInstantLockProof,
            InsufficientBalance { available: u64, required: u64 },
        }

        let kind: Option<ConsensusKind> = {
            let consensus_error = match &error {
                SdkError::StateTransitionBroadcastError(broadcast_err) => {
                    broadcast_err.cause.as_ref()
                }
                SdkError::Protocol(ProtocolError::ConsensusError(ce)) => Some(ce.as_ref()),
                _ => None,
            };

            consensus_error
                .and_then(|ce| match ce {
                    ConsensusError::StateError(
                        StateError::DuplicatedIdentityPublicKeyStateError(_),
                    ) => Some(ConsensusKind::DuplicateKey),
                    ConsensusError::StateError(
                        StateError::DuplicatedIdentityPublicKeyIdStateError(_),
                    ) => Some(ConsensusKind::DuplicateKeyId),
                    ConsensusError::StateError(
                        StateError::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError(e),
                    ) => Some(ConsensusKind::ContractBoundsConflict(
                        e.contract_id().to_string(Encoding::Base58),
                    )),
                    ConsensusError::StateError(StateError::IdentityInsufficientBalanceError(e)) => {
                        Some(ConsensusKind::InsufficientBalance {
                            available: e.balance(),
                            required: e.required_balance(),
                        })
                    }
                    ConsensusError::BasicError(
                        BasicError::InvalidInstantAssetLockProofSignatureError(_),
                    ) => Some(ConsensusKind::InvalidInstantLockProof),
                    _ => None,
                })
                .or_else(|| {
                    if let SdkError::StateTransitionBroadcastError(broadcast_err) = &error
                        && broadcast_err.cause.is_none()
                    {
                        let msg = broadcast_err.message.to_lowercase();
                        if msg.contains("duplicate") {
                            return Some(ConsensusKind::DuplicateKey);
                        }
                    }
                    None
                })
        };

        let boxed = Box::new(error);
        match kind {
            Some(ConsensusKind::DuplicateKey) => TaskError::DuplicateIdentityPublicKey {
                source_error: boxed,
            },
            Some(ConsensusKind::DuplicateKeyId) => TaskError::DuplicateIdentityPublicKeyId {
                source_error: boxed,
            },
            Some(ConsensusKind::ContractBoundsConflict(contract_id)) => {
                TaskError::IdentityPublicKeyContractBoundsConflict {
                    contract_id,
                    source_error: boxed,
                }
            }
            Some(ConsensusKind::InvalidInstantLockProof) => {
                TaskError::AssetLockInstantLockProofInvalid {
                    source_error: boxed,
                }
            }
            Some(ConsensusKind::InsufficientBalance {
                available,
                required,
            }) => TaskError::IdentityInsufficientBalance {
                available,
                required,
                source_error: boxed,
            },
            None => TaskError::SdkError {
                source_error: boxed,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::consensus::basic::identity::InvalidInstantAssetLockProofSignatureError;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_id_state_error::DuplicatedIdentityPublicKeyIdStateError;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_state_error::DuplicatedIdentityPublicKeyStateError;
    use dash_sdk::dpp::consensus::state::identity::IdentityInsufficientBalanceError;
    use dash_sdk::dpp::consensus::state::identity::identity_public_key_already_exists_for_unique_contract_bounds_error::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError;
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::platform::Identifier;

    #[test]
    fn rpc_http_401_converts_to_core_rpc_auth_failed() {
        let http_err = dashcore_rpc::jsonrpc::simple_http::Error::HttpErrorCode(401);
        let transport_err = dashcore_rpc::jsonrpc::error::Error::Transport(Box::new(http_err));
        let err: TaskError = dashcore_rpc::Error::JsonRpc(transport_err).into();
        assert!(
            matches!(err, TaskError::CoreRpcAuthFailed),
            "Expected CoreRpcAuthFailed, got: {err:?}"
        );
    }

    #[test]
    fn rpc_http_401_display_mentions_credentials() {
        let msg = TaskError::CoreRpcAuthFailed.to_string();
        assert!(msg.contains("credentials"));
        assert!(msg.contains("RPC password"));
    }

    #[test]
    fn is_rpc_auth_error_detects_401() {
        let http_err = dashcore_rpc::jsonrpc::simple_http::Error::HttpErrorCode(401);
        let transport_err = dashcore_rpc::jsonrpc::error::Error::Transport(Box::new(http_err));
        let rpc_err = dashcore_rpc::Error::JsonRpc(transport_err);
        assert!(is_rpc_auth_error(&rpc_err));
    }

    #[test]
    fn is_rpc_auth_error_ignores_other_http_codes() {
        let http_err = dashcore_rpc::jsonrpc::simple_http::Error::HttpErrorCode(403);
        let transport_err = dashcore_rpc::jsonrpc::error::Error::Transport(Box::new(http_err));
        let rpc_err = dashcore_rpc::Error::JsonRpc(transport_err);
        assert!(!is_rpc_auth_error(&rpc_err));
    }

    #[test]
    fn is_rpc_auth_error_ignores_rpc_errors() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -1,
            message: "Some error".to_string(),
            data: None,
        };
        let err = dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err));
        assert!(!is_rpc_auth_error(&err));
    }

    #[test]
    fn display_message_is_user_friendly() {
        let msg = TaskError::CoreWalletNotConfigured.to_string();
        assert!(msg.contains("Wallets screen"));
        assert!(msg.contains("refresh"));
    }

    #[test]
    fn must_retry_displays_inner_message() {
        let err = TaskError::MustRetry("Auto-detected Core wallet 'mywallet'".to_string());
        assert_eq!(err.to_string(), "Auto-detected Core wallet 'mywallet'");
    }

    #[test]
    fn rpc_error_code_neg19_converts_to_core_wallet_not_configured() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -19,
            message: "Wallet file not specified".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured),
            "Expected CoreWalletNotConfigured, got: {err:?}"
        );
    }

    #[test]
    fn other_rpc_error_converts_to_core_rpc() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -1,
            message: "Some other error".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::CoreRpc { .. }),
            "Expected CoreRpc, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_duplicate_public_key() {
        let consensus =
            ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1, 2]));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey { .. }));
    }

    #[test]
    fn from_sdk_error_duplicate_public_key_id() {
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyIdStateError::new(vec![3]));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(matches!(
            err,
            TaskError::DuplicateIdentityPublicKeyId { .. }
        ));
    }

    #[test]
    fn from_sdk_error_contract_bounds_conflict() {
        let contract_id = Identifier::random();
        let identity_id = Identifier::random();
        let consensus = ConsensusError::from(
            IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError::new(
                identity_id,
                contract_id,
                Purpose::AUTHENTICATION,
                2,
                1,
            ),
        );
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        let expected_contract_id = contract_id.to_string(Encoding::Base58);
        assert!(
            matches!(err, TaskError::IdentityPublicKeyContractBoundsConflict { ref contract_id, .. } if *contract_id == expected_contract_id)
        );
    }

    #[test]
    fn from_sdk_error_broadcast_cause_duplicate_key() {
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1]));
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40206,
            message: "duplicate key".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = TaskError::from(sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey { .. }));
    }

    #[test]
    fn from_sdk_error_generic_variant_falls_back_to_sdk_error() {
        let sdk_err = SdkError::Generic("connection timeout".to_string());
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(err, TaskError::SdkError { .. }),
            "Expected SdkError, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_broadcast_cause_none_message_duplicate_falls_back_to_duplicate_key() {
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40206,
            message: "DuplicateIdentityPublicKeyStateError".to_string(),
            cause: None,
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(err, TaskError::DuplicateIdentityPublicKey { .. }),
            "Expected DuplicateIdentityPublicKey, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_invalid_instant_lock_proof_via_consensus() {
        let consensus = ConsensusError::from(InvalidInstantAssetLockProofSignatureError::new());
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(err, TaskError::AssetLockInstantLockProofInvalid { .. }),
            "Expected AssetLockInstantLockProofInvalid, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_invalid_instant_lock_proof_via_broadcast() {
        let consensus = ConsensusError::from(InvalidInstantAssetLockProofSignatureError::new());
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40001,
            message: "instant lock proof invalid".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(err, TaskError::AssetLockInstantLockProofInvalid { .. }),
            "Expected AssetLockInstantLockProofInvalid, got: {err:?}"
        );
    }

    #[test]
    fn is_instant_lock_proof_invalid_detects_broadcast_error() {
        let consensus = ConsensusError::from(InvalidInstantAssetLockProofSignatureError::new());
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40001,
            message: "instant lock proof invalid".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        assert!(is_instant_lock_proof_invalid(&sdk_err));
    }

    #[test]
    fn is_instant_lock_proof_invalid_rejects_unrelated_error() {
        let sdk_err = SdkError::Generic("connection timeout".to_string());
        assert!(!is_instant_lock_proof_invalid(&sdk_err));
    }

    #[test]
    fn display_message_for_instant_lock_proof_invalid() {
        let consensus = ConsensusError::from(InvalidInstantAssetLockProofSignatureError::new());
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        let msg = err.to_string();
        assert!(msg.contains("could not be verified instantly"));
        assert!(msg.contains("included in a block"));
    }

    #[test]
    fn is_rpc_connection_error_detects_socket_error() {
        let socket_err = dashcore_rpc::jsonrpc::simple_http::Error::SocketError(
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused"),
        );
        let transport_err = dashcore_rpc::jsonrpc::error::Error::Transport(Box::new(socket_err));
        let rpc_err = dashcore_rpc::Error::JsonRpc(transport_err);
        assert!(is_rpc_connection_error(&rpc_err));
    }

    #[test]
    fn is_rpc_connection_error_ignores_http_error_codes() {
        let http_err = dashcore_rpc::jsonrpc::simple_http::Error::HttpErrorCode(500);
        let transport_err = dashcore_rpc::jsonrpc::error::Error::Transport(Box::new(http_err));
        let rpc_err = dashcore_rpc::Error::JsonRpc(transport_err);
        assert!(!is_rpc_connection_error(&rpc_err));
    }

    #[test]
    fn is_rpc_connection_error_ignores_rpc_errors() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -1,
            message: "Some error".to_string(),
            data: None,
        };
        let err = dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err));
        assert!(!is_rpc_connection_error(&err));
    }

    #[test]
    fn from_sdk_error_insufficient_balance_via_consensus() {
        let identity_id = Identifier::random();
        let consensus = ConsensusError::from(IdentityInsufficientBalanceError::new(
            identity_id,
            12_656_420,
            42_332_820,
        ));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(
                err,
                TaskError::IdentityInsufficientBalance {
                    available: 12_656_420,
                    required: 42_332_820,
                    ..
                }
            ),
            "Expected IdentityInsufficientBalance, got: {err:?}"
        );
    }

    #[test]
    fn from_sdk_error_insufficient_balance_via_broadcast() {
        let identity_id = Identifier::random();
        let consensus =
            ConsensusError::from(IdentityInsufficientBalanceError::new(identity_id, 100, 500));
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40200,
            message: "insufficient balance".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = TaskError::from(sdk_err);
        assert!(
            matches!(
                err,
                TaskError::IdentityInsufficientBalance {
                    available: 100,
                    required: 500,
                    ..
                }
            ),
            "Expected IdentityInsufficientBalance, got: {err:?}"
        );
    }

    #[test]
    fn insufficient_balance_display_includes_amounts_and_action() {
        let identity_id = Identifier::random();
        let consensus = ConsensusError::from(IdentityInsufficientBalanceError::new(
            identity_id,
            12_656_420,
            42_332_820,
        ));
        let sdk_err = SdkError::from(consensus);
        let err = TaskError::from(sdk_err);
        let msg = err.to_string();
        assert!(
            msg.contains("12656420"),
            "Expected available balance in message, got: {msg}"
        );
        assert!(
            msg.contains("42332820"),
            "Expected required balance in message, got: {msg}"
        );
        assert!(
            msg.contains("top up"),
            "Expected actionable guidance in message, got: {msg}"
        );
    }

    #[test]
    fn connection_failed_display_includes_url() {
        let socket_err = dashcore_rpc::jsonrpc::simple_http::Error::SocketError(
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused"),
        );
        let err = TaskError::CoreRpcConnectionFailed {
            url: "127.0.0.1:9998".to_string(),
            source: dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Transport(
                Box::new(socket_err),
            )),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("127.0.0.1:9998"),
            "Expected URL in message, got: {msg}"
        );
        assert!(msg.contains("Dash Core"));
        assert!(msg.contains("network settings"));
    }
}
