use dash_sdk::platform::Identifier;
use thiserror::Error;

/// Comprehensive error types for DashPay operations
#[derive(Error, Debug, Clone, PartialEq)]
pub enum DashPayError {
    // Contact Request Errors
    #[error("Identity not found: {identity_id}")]
    IdentityNotFound { identity_id: Identifier },

    #[error("Username '{username}' could not be resolved via DPNS")]
    UsernameResolutionFailed { username: String },

    #[error("Key index {key_id} not found in identity {identity_id}")]
    KeyNotFound {
        key_id: u32,
        identity_id: Identifier,
    },

    #[error("Key index {key_id} is disabled in identity {identity_id}")]
    KeyDisabled {
        key_id: u32,
        identity_id: Identifier,
    },

    #[error("Key index {key_id} has unsuitable type {key_type:?} for {operation}")]
    UnsuitableKeyType {
        key_id: u32,
        key_type: String,
        operation: String,
    },

    #[error("ECDH key generation failed: {reason}")]
    EcdhFailed { reason: String },

    #[error("Encryption failed: {reason}")]
    EncryptionFailed { reason: String },

    #[error("Decryption failed: {reason}")]
    DecryptionFailed { reason: String },

    // Document/Platform Errors
    #[error("Failed to create contact request document: {reason}")]
    DocumentCreationFailed { reason: String },

    #[error("Failed to broadcast state transition: {reason}")]
    BroadcastFailed { reason: String },

    #[error("Document query failed: {reason}")]
    QueryFailed { reason: String },

    #[error("Invalid document structure: {reason}")]
    InvalidDocument { reason: String },

    // Validation Errors
    #[error("Core height {height} is invalid (current: {current:?}): {reason}")]
    InvalidCoreHeight {
        height: u32,
        current: Option<u32>,
        reason: String,
    },

    #[error("Account reference {account} is invalid: {reason}")]
    InvalidAccountReference { account: u32, reason: String },

    #[error("Contact request validation failed: {errors:?}")]
    ValidationFailed { errors: Vec<String> },

    // Auto Accept Proof Errors
    #[error("Invalid QR code format: {reason}")]
    InvalidQrCode { reason: String },

    #[error("QR code expired at {expired_at}, current time: {current_time}")]
    QrCodeExpired { expired_at: u64, current_time: u64 },

    #[error("Auto-accept proof verification failed: {reason}")]
    ProofVerificationFailed { reason: String },

    // Network/SDK Errors
    #[error("Platform query failed: {reason}")]
    PlatformError { reason: String },

    #[error("Network connection failed: {reason}")]
    NetworkError { reason: String },

    #[error("SDK operation failed: {reason}")]
    SdkError { reason: String },

    // User Input Errors
    #[error("Invalid username format: {username}")]
    InvalidUsername { username: String },

    #[error("Account label too long: {length} chars (max: {max})")]
    AccountLabelTooLong { length: usize, max: usize },

    #[error("Missing required field: {field}")]
    MissingField { field: String },

    // Contact Info Errors
    #[error("Contact info not found for contact {contact_id}")]
    ContactInfoNotFound { contact_id: Identifier },

    #[error("Contact info decryption failed for contact {contact_id}: {reason}")]
    ContactInfoDecryptionFailed {
        contact_id: Identifier,
        reason: String,
    },

    // General Errors
    #[error("Internal error: {message}")]
    Internal { message: String },

    #[error("Operation not supported: {operation}")]
    NotSupported { operation: String },

    #[error("Rate limit exceeded for operation: {operation}")]
    RateLimited { operation: String },
}

impl DashPayError {
    /// Convert to user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            DashPayError::UsernameResolutionFailed { username } => {
                format!(
                    "Username '{}' not found. Please check the spelling.",
                    username
                )
            }
            DashPayError::IdentityNotFound { .. } => {
                "Contact not found. They may not be registered on Dash Platform.".to_string()
            }
            DashPayError::InvalidQrCode { .. } => {
                "Invalid QR code. Please scan a valid DashPay contact QR code.".to_string()
            }
            DashPayError::QrCodeExpired { .. } => {
                "QR code has expired. Please ask for a new one.".to_string()
            }
            DashPayError::NetworkError { .. } => {
                "Network connection error. Please check your internet connection.".to_string()
            }
            DashPayError::ValidationFailed { errors } => {
                if errors.len() == 1 {
                    format!("Validation error: {}", errors[0])
                } else {
                    format!("Multiple validation errors: {}", errors.join(", "))
                }
            }
            DashPayError::AccountLabelTooLong { max, .. } => {
                format!(
                    "Account label too long. Maximum {} characters allowed.",
                    max
                )
            }
            DashPayError::InvalidUsername { .. } => {
                "Invalid username format. Usernames must end with '.dash'.".to_string()
            }
            DashPayError::RateLimited { .. } => {
                "Too many requests. Please wait a moment before trying again.".to_string()
            }
            DashPayError::Internal { message } => {
                // Show the actual internal error message
                message.clone()
            }
            _ => "An error occurred. Please try again.".to_string(),
        }
    }

    /// Check if error is recoverable (user can retry)
    pub fn is_recoverable(&self) -> bool {
        match self {
            DashPayError::NetworkError { .. } => true,
            DashPayError::PlatformError { .. } => true,
            DashPayError::RateLimited { .. } => true,
            DashPayError::BroadcastFailed { .. } => true,
            DashPayError::QueryFailed { .. } => true,
            _ => false,
        }
    }

    /// Check if error requires user action (not a system error)
    pub fn requires_user_action(&self) -> bool {
        match self {
            DashPayError::UsernameResolutionFailed { .. } => true,
            DashPayError::InvalidQrCode { .. } => true,
            DashPayError::QrCodeExpired { .. } => true,
            DashPayError::ValidationFailed { .. } => true,
            DashPayError::AccountLabelTooLong { .. } => true,
            DashPayError::InvalidUsername { .. } => true,
            DashPayError::MissingField { .. } => true,
            _ => false,
        }
    }
}

/// Result type for DashPay operations
pub type DashPayResult<T> = Result<T, DashPayError>;

/// Helper to convert string errors to DashPayError
impl From<String> for DashPayError {
    fn from(error: String) -> Self {
        DashPayError::Internal { message: error }
    }
}

/// Trait for converting various SDK errors to DashPayError
pub trait ToDashPayError<T> {
    fn to_dashpay_error(self, context: &str) -> DashPayResult<T>;
}

impl<T> ToDashPayError<T> for Result<T, dash_sdk::Error> {
    fn to_dashpay_error(self, context: &str) -> DashPayResult<T> {
        self.map_err(|e| DashPayError::SdkError {
            reason: format!("{}: {}", context, e),
        })
    }
}

impl<T> ToDashPayError<T> for Result<T, String> {
    fn to_dashpay_error(self, context: &str) -> DashPayResult<T> {
        self.map_err(|e| DashPayError::Internal {
            message: format!("{}: {}", context, e),
        })
    }
}

/// Helper to create validation errors
pub fn validation_error(errors: Vec<String>) -> DashPayError {
    DashPayError::ValidationFailed { errors }
}

/// Helper to create network errors
pub fn network_error(reason: impl Into<String>) -> DashPayError {
    DashPayError::NetworkError {
        reason: reason.into(),
    }
}

/// Helper to create platform errors
pub fn platform_error(reason: impl Into<String>) -> DashPayError {
    DashPayError::PlatformError {
        reason: reason.into(),
    }
}
