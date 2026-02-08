//! Crate-level error type hierarchy for backend tasks.
//!
//! This module defines structured error types that replace raw `String` errors
//! throughout the backend task system. Each domain module has its own error type,
//! and `BackendTaskError` wraps them all at the top level.
//!
//! # Migration Strategy
//!
//! All error types implement `From<String>` to allow gradual migration:
//! existing code that returns `Err("some message".to_string())` works
//! unchanged via `?`. Over time, individual functions can adopt richer
//! error variants for better user-facing messages and recoverability.

use thiserror::Error;

/// Top-level error type for all backend task operations.
///
/// Each domain has a dedicated error variant wrapping a module-specific error type.
/// The `From<String>` impl allows gradual migration from raw string errors.
#[derive(Debug, Clone, Error)]
pub enum BackendTaskError {
    #[error("{0}")]
    Identity(#[from] IdentityError),

    #[error("{0}")]
    Wallet(#[from] WalletError),

    #[error("{0}")]
    Core(#[from] CoreError),

    #[error("{0}")]
    Contract(#[from] ContractError),

    #[error("{0}")]
    Document(#[from] DocumentError),

    #[error("{0}")]
    Token(#[from] TokenError),

    #[error("{0}")]
    Contest(#[from] ContestError),

    #[error("{0}")]
    DashPay(#[from] DashPayTaskError),

    #[error("{0}")]
    Platform(#[from] PlatformError),

    #[error("{0}")]
    MnList(#[from] MnListError),

    #[error("{0}")]
    GroveSTARK(#[from] GroveSTARKError),

    #[error("{0}")]
    System(#[from] SystemError),

    #[error("{0}")]
    Broadcast(#[from] BroadcastError),

    /// Catch-all for unstructured string errors during migration.
    /// New code should use a domain-specific variant instead.
    #[error("{0}")]
    Generic(String),
}

impl From<String> for BackendTaskError {
    fn from(s: String) -> Self {
        BackendTaskError::Generic(s)
    }
}

impl From<&str> for BackendTaskError {
    fn from(s: &str) -> Self {
        BackendTaskError::Generic(s.to_string())
    }
}

impl BackendTaskError {
    /// Returns a user-friendly error message suitable for display in the UI.
    pub fn user_message(&self) -> String {
        match self {
            BackendTaskError::Identity(e) => e.user_message(),
            BackendTaskError::Wallet(e) => e.user_message(),
            BackendTaskError::Core(e) => e.user_message(),
            BackendTaskError::Contract(e) => e.user_message(),
            BackendTaskError::Document(e) => e.user_message(),
            BackendTaskError::Token(e) => e.user_message(),
            BackendTaskError::Contest(e) => e.user_message(),
            BackendTaskError::DashPay(e) => e.user_message(),
            BackendTaskError::Platform(e) => e.user_message(),
            BackendTaskError::MnList(e) => e.user_message(),
            BackendTaskError::GroveSTARK(e) => e.user_message(),
            BackendTaskError::System(e) => e.user_message(),
            BackendTaskError::Broadcast(e) => e.user_message(),
            BackendTaskError::Generic(msg) => classify_generic_error(msg),
        }
    }

    /// Returns the raw technical details of the error for debugging.
    pub fn technical_details(&self) -> String {
        self.to_string()
    }

    /// Whether the user can retry this operation and it might succeed.
    pub fn is_recoverable(&self) -> bool {
        match self {
            BackendTaskError::Identity(e) => e.is_recoverable(),
            BackendTaskError::Wallet(e) => e.is_recoverable(),
            BackendTaskError::Core(e) => e.is_recoverable(),
            BackendTaskError::Contract(e) => e.is_recoverable(),
            BackendTaskError::Document(e) => e.is_recoverable(),
            BackendTaskError::Token(e) => e.is_recoverable(),
            BackendTaskError::Contest(e) => e.is_recoverable(),
            BackendTaskError::DashPay(e) => e.is_recoverable(),
            BackendTaskError::Platform(e) => e.is_recoverable(),
            BackendTaskError::MnList(e) => e.is_recoverable(),
            BackendTaskError::GroveSTARK(e) => e.is_recoverable(),
            BackendTaskError::System(e) => e.is_recoverable(),
            BackendTaskError::Broadcast(e) => e.is_recoverable(),
            BackendTaskError::Generic(msg) => is_generic_error_recoverable(msg),
        }
    }
}

// --- Per-module error types ---
// Each follows the same pattern:
// - Domain-specific variants for common error cases
// - A `Generic(String)` catch-all for migration
// - `From<String>` impl for compatibility
// - `user_message()` and `is_recoverable()` methods

/// Errors from identity operations (registration, top-up, key management, etc.)
#[derive(Debug, Clone, Error)]
pub enum IdentityError {
    #[error("Identity not found: {0}")]
    NotFound(String),

    #[error("Identity registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Identity top-up failed: {0}")]
    TopUpFailed(String),

    #[error("Key operation failed: {0}")]
    KeyError(String),

    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),

    #[error("DPNS name registration failed: {0}")]
    DpnsError(String),

    #[error("Identity transfer failed: {0}")]
    TransferFailed(String),

    #[error("Identity withdrawal failed: {0}")]
    WithdrawalFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for IdentityError {
    fn from(s: String) -> Self {
        IdentityError::Generic(s)
    }
}

impl IdentityError {
    pub fn user_message(&self) -> String {
        match self {
            IdentityError::NotFound(_) => {
                "Identity not found on Platform. It may not be registered yet.".to_string()
            }
            IdentityError::InsufficientBalance(_) => {
                "Insufficient balance for this operation.".to_string()
            }
            _ => classify_generic_error(&self.to_string()),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, IdentityError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from wallet operations (address generation, platform funding, etc.)
#[derive(Debug, Clone, Error)]
pub enum WalletError {
    #[error("Wallet not found: {0}")]
    NotFound(String),

    #[error("Wallet is locked")]
    Locked,

    #[error("Insufficient funds: {0}")]
    InsufficientFunds(String),

    #[error("Address generation failed: {0}")]
    AddressError(String),

    #[error("Platform funding failed: {0}")]
    FundingFailed(String),

    #[error("Platform withdrawal failed: {0}")]
    WithdrawalFailed(String),

    #[error("Transfer failed: {0}")]
    TransferFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for WalletError {
    fn from(s: String) -> Self {
        WalletError::Generic(s)
    }
}

impl WalletError {
    pub fn user_message(&self) -> String {
        match self {
            WalletError::Locked => "Wallet is locked. Please unlock it first.".to_string(),
            WalletError::InsufficientFunds(_) => {
                "Insufficient funds for this transaction.".to_string()
            }
            _ => classify_generic_error(&self.to_string()),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, WalletError::Locked)
            || matches!(self, WalletError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from Core RPC operations (payments, asset locks, refresh, etc.)
#[derive(Debug, Clone, Error)]
pub enum CoreError {
    #[error("Core RPC connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Asset lock failed: {0}")]
    AssetLockFailed(String),

    #[error("Wallet refresh failed: {0}")]
    RefreshFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for CoreError {
    fn from(s: String) -> Self {
        CoreError::Generic(s)
    }
}

impl CoreError {
    pub fn user_message(&self) -> String {
        match self {
            CoreError::ConnectionFailed(_) => {
                "Cannot connect to Dash Core. Check that it is running and configured.".to_string()
            }
            _ => classify_generic_error(&self.to_string()),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            CoreError::ConnectionFailed(_) | CoreError::RefreshFailed(_)
        ) || matches!(self, CoreError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from contract operations (fetch, register, update, etc.)
#[derive(Debug, Clone, Error)]
pub enum ContractError {
    #[error("Contract not found: {0}")]
    NotFound(String),

    #[error("Contract registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Contract update failed: {0}")]
    UpdateFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for ContractError {
    fn from(s: String) -> Self {
        ContractError::Generic(s)
    }
}

impl ContractError {
    pub fn user_message(&self) -> String {
        match self {
            ContractError::NotFound(_) => {
                "Contract not found on Platform. Check the ID and try again.".to_string()
            }
            _ => classify_generic_error(&self.to_string()),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, ContractError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from document operations (query, broadcast, etc.)
#[derive(Debug, Clone, Error)]
pub enum DocumentError {
    #[error("Document query failed: {0}")]
    QueryFailed(String),

    #[error("Document broadcast failed: {0}")]
    BroadcastFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for DocumentError {
    fn from(s: String) -> Self {
        DocumentError::Generic(s)
    }
}

impl DocumentError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, DocumentError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from token operations (mint, burn, freeze, transfer, etc.)
#[derive(Debug, Clone, Error)]
pub enum TokenError {
    #[error("Token not found: {0}")]
    NotFound(String),

    #[error("Token operation failed: {0}")]
    OperationFailed(String),

    #[error("Insufficient token balance: {0}")]
    InsufficientBalance(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for TokenError {
    fn from(s: String) -> Self {
        TokenError::Generic(s)
    }
}

impl TokenError {
    pub fn user_message(&self) -> String {
        match self {
            TokenError::NotFound(_) => "Token not found on Platform.".to_string(),
            TokenError::InsufficientBalance(_) => {
                "Insufficient token balance for this operation.".to_string()
            }
            _ => classify_generic_error(&self.to_string()),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, TokenError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from contested name / DPNS voting operations.
#[derive(Debug, Clone, Error)]
pub enum ContestError {
    #[error("Contest query failed: {0}")]
    QueryFailed(String),

    #[error("Vote failed: {0}")]
    VoteFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for ContestError {
    fn from(s: String) -> Self {
        ContestError::Generic(s)
    }
}

impl ContestError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, ContestError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from DashPay operations (profiles, contacts, payments, etc.)
#[derive(Debug, Clone, Error)]
pub enum DashPayTaskError {
    #[error("{0}")]
    DashPay(#[from] super::dashpay::errors::DashPayError),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for DashPayTaskError {
    fn from(s: String) -> Self {
        DashPayTaskError::Generic(s)
    }
}

impl DashPayTaskError {
    pub fn user_message(&self) -> String {
        match self {
            DashPayTaskError::DashPay(e) => e.user_message(),
            DashPayTaskError::Generic(msg) => classify_generic_error(msg),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        match self {
            DashPayTaskError::DashPay(e) => e.is_recoverable(),
            DashPayTaskError::Generic(msg) => is_generic_error_recoverable(msg),
        }
    }
}

/// Errors from Platform info queries.
#[derive(Debug, Clone, Error)]
pub enum PlatformError {
    #[error("Platform query failed: {0}")]
    QueryFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for PlatformError {
    fn from(s: String) -> Self {
        PlatformError::Generic(s)
    }
}

impl PlatformError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        true // Platform queries are generally retriable
    }
}

/// Errors from masternode list diff operations.
#[derive(Debug, Clone, Error)]
pub enum MnListError {
    #[error("Masternode list query failed: {0}")]
    QueryFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for MnListError {
    fn from(s: String) -> Self {
        MnListError::Generic(s)
    }
}

impl MnListError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        true
    }
}

/// Errors from GroveSTARK proof operations.
#[derive(Debug, Clone, Error)]
pub enum GroveSTARKError {
    #[error("Proof generation failed: {0}")]
    GenerationFailed(String),

    #[error("Proof verification failed: {0}")]
    VerificationFailed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for GroveSTARKError {
    fn from(s: String) -> Self {
        GroveSTARKError::Generic(s)
    }
}

impl GroveSTARKError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(self, GroveSTARKError::Generic(msg) if is_generic_error_recoverable(msg))
    }
}

/// Errors from system tasks (theme, settings, etc.)
#[derive(Debug, Clone, Error)]
pub enum SystemError {
    #[error("{0}")]
    Generic(String),
}

impl From<String> for SystemError {
    fn from(s: String) -> Self {
        SystemError::Generic(s)
    }
}

impl SystemError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        false
    }
}

/// Errors from state transition broadcast operations.
#[derive(Debug, Clone, Error)]
pub enum BroadcastError {
    #[error("State transition broadcast failed: {0}")]
    Failed(String),

    #[error("{0}")]
    Generic(String),
}

impl From<String> for BroadcastError {
    fn from(s: String) -> Self {
        BroadcastError::Generic(s)
    }
}

impl BroadcastError {
    pub fn user_message(&self) -> String {
        classify_generic_error(&self.to_string())
    }

    pub fn is_recoverable(&self) -> bool {
        is_generic_error_recoverable(&self.to_string())
    }
}

// --- Shared error classification helpers ---

/// Classify a raw error string into a user-friendly message by pattern matching
/// on common SDK/Platform error patterns.
fn classify_generic_error(msg: &str) -> String {
    let lower = msg.to_lowercase();

    if lower.contains("transport") && lower.contains("unavailable") {
        return "Connection to Platform failed. Check your network connection.".to_string();
    }
    if lower.contains("transport") && lower.contains("deadline") {
        return "Request timed out. The Platform may be busy. Try again.".to_string();
    }
    if lower.contains("insufficient") {
        return "Insufficient funds for this operation.".to_string();
    }
    if lower.contains("not found") && lower.contains("identity") {
        return "Identity not found on Platform.".to_string();
    }
    if lower.contains("already exists") {
        return "This item already exists on Platform.".to_string();
    }
    if lower.contains("lock") && lower.contains("poisoned") {
        return "Internal error: a lock was poisoned. Please restart the application.".to_string();
    }
    if lower.contains("connection refused") || lower.contains("connect error") {
        return "Connection refused. Check that Dash Core is running.".to_string();
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return "Operation timed out. Please try again.".to_string();
    }

    // Fall back to the raw message — it's already somewhat readable
    msg.to_string()
}

/// Check if a raw error string represents a recoverable (retriable) error.
fn is_generic_error_recoverable(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("transport")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("unavailable")
        || lower.contains("deadline")
        || lower.contains("connection refused")
        || lower.contains("connect error")
        || lower.contains("rate limit")
}
