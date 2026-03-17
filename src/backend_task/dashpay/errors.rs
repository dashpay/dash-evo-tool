use dash_sdk::dpp::dashcore::secp256k1;
use dash_sdk::platform::Identifier;
use thiserror::Error;

/// Comprehensive error types for DashPay operations
#[derive(Error, Debug)]
pub enum DashPayError {
    // Contact Request Errors
    #[error("This contact could not be found on the network. Please check the ID and try again.")]
    IdentityNotFound { identity_id: Identifier },

    #[error("Username '{username}' was not found. Please check the spelling and try again.")]
    UsernameResolutionFailed { username: String },

    #[error("A required security key is missing. Please refresh your identity and retry.")]
    KeyNotFound {
        key_id: u32,
        identity_id: Identifier,
    },

    #[error("A required security key has been disabled. Please refresh your identity and retry.")]
    KeyDisabled {
        key_id: u32,
        identity_id: Identifier,
    },

    #[error(
        "A security key cannot be used for this operation. Please refresh your identity and retry."
    )]
    UnsuitableKeyType {
        key_id: u32,
        key_type: String,
        operation: String,
    },

    #[error(
        "Your identity is missing an encryption key required for contacts. Please add a compatible encryption key."
    )]
    MissingEncryptionKey,

    #[error(
        "Your identity is missing a decryption key required for contacts. Please add a compatible decryption key."
    )]
    MissingDecryptionKey,

    #[error("Could not establish a secure connection with this contact. Please retry.")]
    EcdhFailed { reason: String },

    #[error("Could not encrypt the message. Please retry.")]
    EncryptionFailed { reason: String },

    #[error("Could not decrypt the message. Please retry.")]
    DecryptionFailed { reason: String },

    // Document/Platform Errors
    #[error("Could not send the contact request. Please retry.")]
    DocumentCreationFailed { reason: String },

    #[error("Could not send this request to the network. Please retry.")]
    BroadcastFailed { reason: String },

    #[error(
        "Could not retrieve the requested information. Please check your connection and retry."
    )]
    QueryFailed { reason: String },

    #[error("The received data has an unexpected format. Please retry or update the application.")]
    InvalidDocument { reason: String },

    // Validation Errors
    #[error("The network data could not be verified. Please retry in a few moments.")]
    InvalidCoreHeight {
        height: u32,
        current: Option<u32>,
        reason: String,
    },

    #[error("The account reference is not valid. Please check the details and retry.")]
    InvalidAccountReference { account: u32, reason: String },

    #[error("The contact request could not be verified. Please check the details and try again.")]
    ValidationFailed { errors: Vec<String> },

    // Auto Accept Proof Errors
    #[error("The QR code format is not recognized. Please scan a valid contact QR code.")]
    InvalidQrCode { reason: String },

    #[error("This QR code has expired. Please ask for a new one.")]
    QrCodeExpired { expired_at: u64, current_time: u64 },

    #[error("The automatic verification could not be completed. Please retry.")]
    ProofVerificationFailed { reason: String },

    // Network/SDK Errors
    #[error("Could not reach the network. Please check your connection and retry.")]
    PlatformError { reason: String },

    #[error("Network connection failed. Please check your internet connection and retry.")]
    NetworkError { reason: String },

    #[error("An unexpected error occurred while communicating with the network. Please retry.")]
    SdkError {
        #[source]
        source: Box<dash_sdk::Error>,
    },

    // User Input Errors
    #[error("The username format is not valid. Usernames must end with '.dash'.")]
    InvalidUsername { username: String },

    #[error("The account label is too long. Please use {max} characters or fewer.")]
    AccountLabelTooLong { length: usize, max: usize },

    #[error("A required field is missing. Please fill in all fields and try again.")]
    MissingField { field: String },

    // Contact Info Errors
    #[error("Contact information could not be found. Please refresh and try again.")]
    ContactInfoNotFound { contact_id: Identifier },

    #[error("Contact information could not be read. Please refresh and try again.")]
    ContactInfoDecryptionFailed {
        contact_id: Identifier,
        reason: String,
    },

    // General Errors
    #[error("An unexpected error occurred. Please retry.")]
    Internal { message: String },

    #[error("This operation is not available. Please update the application.")]
    NotSupported { operation: String },

    #[error("Too many requests. Please wait a moment and try again.")]
    RateLimited { operation: String },

    /// Failed to build a document query (schema / configuration error).
    #[error("Could not prepare the data request. Please retry or update the application.")]
    QueryCreation {
        /// Description of what query was being built (e.g., "contact requests", "DPNS domain").
        query_target: &'static str,
        #[source]
        source: Box<dash_sdk::Error>,
    },

    /// Failed to parse a cryptographic key (secp256k1).
    #[error("Could not read a cryptographic key. The data may be corrupted.")]
    CryptoKeyParsing {
        #[from]
        source: secp256k1::Error,
    },

    /// Failed to resolve a private key from the identity's key store.
    #[error(
        "Could not find the required private key in your wallet. Try refreshing your identities."
    )]
    PrivateKeyResolution {
        /// Human-readable key purpose (e.g. "ENCRYPTION", "AUTHENTICATION").
        key_purpose: String,
        /// Details about why the lookup failed.
        reason: String,
    },

    /// The identity does not have a suitable authentication key for this operation.
    #[error(
        "This identity is missing an authentication key required for this operation. Please add an authentication key."
    )]
    MissingAuthenticationKey,

    /// A contact request has already been sent to this recipient.
    #[error("You have already sent a contact request to '{to}'. Please wait for them to respond.")]
    ContactRequestAlreadySent { to: String },
}

impl DashPayError {
    // TODO: `user_message()` is largely redundant now that Display messages are
    // user-friendly. Consider removing it once the two UI callsites
    // (contact_requests.rs:617, add_contact_screen.rs:350) are migrated to use
    // Display directly.

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
                message.clone()
            }
            DashPayError::MissingEncryptionKey => {
                "Your identity is missing an encryption key required for contacts. Please add a compatible encryption key.".to_string()
            }
            DashPayError::MissingDecryptionKey => {
                "Your identity is missing a decryption key required for contacts. Please add a compatible decryption key.".to_string()
            }
            _ => "An error occurred. Please try again.".to_string(),
        }
    }

    /// Check if error is recoverable (user can retry)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            DashPayError::NetworkError { .. }
                | DashPayError::PlatformError { .. }
                | DashPayError::RateLimited { .. }
                | DashPayError::BroadcastFailed { .. }
                | DashPayError::QueryFailed { .. }
        )
    }

    /// Check if error requires user action (not a system error)
    pub fn requires_user_action(&self) -> bool {
        matches!(
            self,
            DashPayError::UsernameResolutionFailed { .. }
                | DashPayError::InvalidQrCode { .. }
                | DashPayError::QrCodeExpired { .. }
                | DashPayError::ValidationFailed { .. }
                | DashPayError::AccountLabelTooLong { .. }
                | DashPayError::InvalidUsername { .. }
                | DashPayError::MissingField { .. }
                | DashPayError::MissingEncryptionKey
                | DashPayError::MissingDecryptionKey
        )
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
    fn to_dashpay_error(self, _context: &str) -> DashPayResult<T> {
        self.map_err(|e| DashPayError::SdkError {
            source: Box::new(e),
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
