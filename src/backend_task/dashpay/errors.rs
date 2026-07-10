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

    #[error(
        "Your identity is missing an encryption key required for contacts. Please add a compatible encryption key."
    )]
    MissingEncryptionKey,

    #[error(
        "Your identity is missing a decryption key required for contacts. Please add a compatible decryption key."
    )]
    MissingDecryptionKey,

    // Document/Platform Errors
    #[error("The received data has an unexpected format. Please retry or update the application.")]
    InvalidDocument { reason: String },

    // Validation Errors
    #[error("The contact request could not be verified. Please check the details and try again.")]
    ValidationFailed { errors: Vec<String> },

    // Auto Accept Proof Errors
    #[error("The QR code format is not recognized. Please scan a valid contact QR code.")]
    InvalidQrCode { reason: String },

    #[error("This QR code has expired. Please ask for a new one.")]
    QrCodeExpired { expired_at: u64, current_time: u64 },

    // Network/SDK Errors
    #[error("Network connection failed. Please check your internet connection and retry.")]
    NetworkError,

    #[error("An unexpected error occurred while communicating with the network. Please retry.")]
    SdkError {
        #[source]
        source: Box<dash_sdk::Error>,
    },

    // User Input Errors
    #[error("You cannot send a contact request to yourself.")]
    CannotContactSelf,

    #[error("The username format is not valid. Usernames must end with '.dash'.")]
    InvalidUsername { username: String },

    #[error("The account label is too long. Please use {max} characters or fewer.")]
    AccountLabelTooLong { length: usize, max: usize },

    #[error("A required field is missing. Please fill in all fields and try again.")]
    MissingField { field: String },

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

    /// Encrypted contact info fields exceed DashPay contract limits.
    #[error("Contact info is too large to save. Try shortening your nickname or note.")]
    ContactInfoValidationFailed { errors: Vec<String> },

    /// DashPay HD key derivation failed.
    #[error("The payment keys for this contact could not be derived. Please retry.")]
    Derivation(#[from] crate::model::dashpay_derivation::DerivationError),

    /// The system clock is set before the Unix epoch, so an expiry time
    /// cannot be computed.
    #[error(
        "Your device clock appears to be incorrect. Please set the correct date and time, then retry."
    )]
    SystemClockInvalid,
}

impl DashPayError {
    /// Check if error is recoverable (user can retry)
    pub fn is_recoverable(&self) -> bool {
        matches!(self, DashPayError::NetworkError)
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
                | DashPayError::ContactInfoValidationFailed { .. }
                | DashPayError::CannotContactSelf
        )
    }
}
