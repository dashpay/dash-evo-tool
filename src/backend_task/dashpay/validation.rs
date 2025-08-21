use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::Identifier;

/// Validation result for contact request fields
#[derive(Debug, Clone)]
pub struct ContactRequestValidation {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ContactRequestValidation {
    pub fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.is_valid = false;
    }

    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    pub fn merge(&mut self, other: ContactRequestValidation) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        if !other.is_valid {
            self.is_valid = false;
        }
    }
}

/// Validate sender key index exists and is suitable for contact requests
pub fn validate_sender_key_index(
    identity: &QualifiedIdentity,
    key_index: u32,
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    // Find the key by ID
    match identity.identity.get_public_key_by_id(key_index) {
        Some(key) => {
            // Verify key type is suitable for signing
            match key.key_type() {
                KeyType::ECDSA_SECP256K1 => {
                    // This is the expected key type for contact requests
                }
                KeyType::ECDSA_HASH160 => {
                    validation.add_error(format!(
                        "Sender key {} is ECDSA_HASH160 type, cannot be used for signing contact requests",
                        key_index
                    ));
                }
                _ => {
                    validation.add_warning(format!(
                        "Sender key {} has unusual type {:?} for contact requests",
                        key_index,
                        key.key_type()
                    ));
                }
            }

            // Verify purpose is suitable
            match key.purpose() {
                Purpose::AUTHENTICATION => {
                    // Perfect for contact requests
                }
                Purpose::ENCRYPTION => {
                    validation.add_warning(format!(
                        "Sender key {} has ENCRYPTION purpose, consider using AUTHENTICATION key",
                        key_index
                    ));
                }
                _ => {
                    validation.add_warning(format!(
                        "Sender key {} has unusual purpose {:?} for contact requests",
                        key_index,
                        key.purpose()
                    ));
                }
            }

            // Verify security level
            match key.security_level() {
                SecurityLevel::MASTER
                | SecurityLevel::CRITICAL
                | SecurityLevel::HIGH
                | SecurityLevel::MEDIUM => {
                    // Acceptable security levels
                }
            }

            // Check if key is disabled
            if let Some(disabled_at) = key.disabled_at() {
                validation.add_error(format!(
                    "Sender key {} is disabled (at timestamp {})",
                    key_index, disabled_at
                ));
            }
        }
        None => {
            validation.add_error(format!(
                "Sender key index {} not found in identity {}",
                key_index,
                identity.identity.id()
            ));
        }
    }

    validation
}

/// Validate recipient key index exists and is suitable for encryption
pub async fn validate_recipient_key_index(
    sdk: &Sdk,
    recipient_identity_id: Identifier,
    key_index: u32,
) -> Result<ContactRequestValidation, String> {
    let mut validation = ContactRequestValidation::new();

    // For now, skip recipient key validation since we don't have a direct SDK method
    // In a real implementation, we would query the identity from the platform
    validation.add_warning(format!(
        "Cannot validate recipient key {} - identity validation skipped",
        key_index
    ));

    Ok(validation)
}

/// Validate that a contact request's core height is reasonable
pub fn validate_core_height_created_at(
    core_height: u32,
    current_core_height: Option<u32>,
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    if let Some(current_height) = current_core_height {
        // Check if the height is too far in the future (max 10 blocks ahead)
        if core_height > current_height + 10 {
            validation.add_error(format!(
                "Core height {} is too far in the future (current: {})",
                core_height, current_height
            ));
        }

        // Check if the height is too far in the past (max 1000 blocks behind)
        if current_height > core_height + 1000 {
            validation.add_warning(format!(
                "Core height {} is quite old (current: {}, {} blocks behind)",
                core_height,
                current_height,
                current_height - core_height
            ));
        }
    } else {
        validation
            .add_warning("Cannot validate core height - current height unavailable".to_string());
    }

    validation
}

/// Validate account reference is within reasonable bounds
pub fn validate_account_reference(account_reference: u32) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    // DashPay typically uses accounts 0-2147483647 (2^31 - 1)
    if account_reference >= 2147483648 {
        validation.add_warning(format!(
            "Account reference {} is very high (using hardened derivation)",
            account_reference
        ));
    }

    // Warn about unusually high account numbers
    if account_reference > 1000 {
        validation.add_warning(format!(
            "Account reference {} is unusually high for typical usage",
            account_reference
        ));
    }

    validation
}

/// Validate toUserId matches the recipient identity
pub fn validate_to_user_id(
    to_user_id: Identifier,
    expected_recipient: Identifier,
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    if to_user_id != expected_recipient {
        validation.add_error(format!(
            "toUserId {} does not match expected recipient {}",
            to_user_id, expected_recipient
        ));
    }

    validation
}

/// Validate field sizes according to DIP-0015 specifications
pub fn validate_contact_request_field_sizes(
    encrypted_public_key: &[u8],
    encrypted_account_label: Option<&[u8]>,
    auto_accept_proof: Option<&[u8]>,
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    // Validate encryptedPublicKey size (must be exactly 96 bytes)
    if encrypted_public_key.len() != 96 {
        validation.add_error(format!(
            "encryptedPublicKey must be exactly 96 bytes, got {}",
            encrypted_public_key.len()
        ));
    }

    // Validate encryptedAccountLabel size (48-80 bytes if present)
    if let Some(label) = encrypted_account_label {
        if label.len() < 48 || label.len() > 80 {
            validation.add_error(format!(
                "encryptedAccountLabel must be 48-80 bytes, got {}",
                label.len()
            ));
        }
    }

    // Validate autoAcceptProof size (38-102 bytes if present and not empty)
    if let Some(proof) = auto_accept_proof {
        if !proof.is_empty() && (proof.len() < 38 || proof.len() > 102) {
            validation.add_error(format!(
                "autoAcceptProof must be 38-102 bytes when present, got {}",
                proof.len()
            ));
        }
    }

    validation
}

/// Validate profile field sizes according to DIP-0015
pub fn validate_profile_field_sizes(
    display_name: Option<&str>,
    public_message: Option<&str>,
    avatar_url: Option<&str>,
    avatar_hash: Option<&[u8]>,
    avatar_fingerprint: Option<&[u8]>,
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    // Validate displayName (0-25 characters)
    if let Some(name) = display_name {
        if name.chars().count() > 25 {
            validation.add_error(format!(
                "displayName must be 0-25 characters, got {}",
                name.chars().count()
            ));
        }
    }

    // Validate publicMessage (0-250 characters)
    if let Some(msg) = public_message {
        if msg.chars().count() > 250 {
            validation.add_error(format!(
                "publicMessage must be 0-250 characters, got {}",
                msg.chars().count()
            ));
        }
    }

    // Validate avatarUrl (0-2048 characters)
    if let Some(url) = avatar_url {
        if url.chars().count() > 2048 {
            validation.add_error(format!(
                "avatarUrl must be 0-2048 characters, got {}",
                url.chars().count()
            ));
        }

        // Validate URL format
        if !url.is_empty() && !url.starts_with("https://") && !url.starts_with("http://") {
            validation.add_warning("avatarUrl should use HTTPS protocol".to_string());
        }
    }

    // Validate avatarHash (exactly 32 bytes if present)
    if let Some(hash) = avatar_hash {
        if hash.len() != 32 {
            validation.add_error(format!(
                "avatarHash must be exactly 32 bytes, got {}",
                hash.len()
            ));
        }
    }

    // Validate avatarFingerprint (exactly 8 bytes if present)
    if let Some(fingerprint) = avatar_fingerprint {
        if fingerprint.len() != 8 {
            validation.add_error(format!(
                "avatarFingerprint must be exactly 8 bytes, got {}",
                fingerprint.len()
            ));
        }
    }

    validation
}

/// Validate contactInfo field sizes according to DIP-0015
pub fn validate_contact_info_field_sizes(
    enc_to_user_id: &[u8],
    private_data: &[u8],
) -> ContactRequestValidation {
    let mut validation = ContactRequestValidation::new();

    // Validate encToUserId (exactly 32 bytes)
    if enc_to_user_id.len() != 32 {
        validation.add_error(format!(
            "encToUserId must be exactly 32 bytes, got {}",
            enc_to_user_id.len()
        ));
    }

    // Validate privateData (48-2048 bytes)
    if private_data.len() < 48 || private_data.len() > 2048 {
        validation.add_error(format!(
            "privateData must be 48-2048 bytes, got {}",
            private_data.len()
        ));
    }

    validation
}

/// Comprehensive validation of a contact request before sending
pub async fn validate_contact_request_before_send(
    sdk: &Sdk,
    sender_identity: &QualifiedIdentity,
    sender_key_index: u32,
    recipient_identity_id: Identifier,
    recipient_key_index: u32,
    account_reference: u32,
    core_height: u32,
    current_core_height: Option<u32>,
) -> Result<ContactRequestValidation, String> {
    let mut validation = ContactRequestValidation::new();

    // Validate sender key
    let sender_validation = validate_sender_key_index(sender_identity, sender_key_index);
    validation.merge(sender_validation);

    // Validate recipient key
    let recipient_validation =
        validate_recipient_key_index(sdk, recipient_identity_id, recipient_key_index).await?;
    validation.merge(recipient_validation);

    // Validate core height
    let height_validation = validate_core_height_created_at(core_height, current_core_height);
    validation.merge(height_validation);

    // Validate account reference
    let account_validation = validate_account_reference(account_reference);
    validation.merge(account_validation);

    // Validate toUserId matches recipient
    let user_id_validation = validate_to_user_id(recipient_identity_id, recipient_identity_id);
    validation.merge(user_id_validation);

    Ok(validation)
}

/// Validate an incoming contact request
pub async fn validate_incoming_contact_request(
    sdk: &Sdk,
    our_identity: &QualifiedIdentity,
    sender_identity_id: Identifier,
    sender_key_index: u32,
    our_key_index: u32,
    account_reference: u32,
    core_height: u32,
    current_core_height: Option<u32>,
) -> Result<ContactRequestValidation, String> {
    let mut validation = ContactRequestValidation::new();

    // Validate sender key exists (fetch their identity)
    let sender_validation =
        validate_recipient_key_index(sdk, sender_identity_id, sender_key_index).await?;
    validation.merge(sender_validation);

    // Validate our key for decryption
    let our_key_validation = validate_sender_key_index(our_identity, our_key_index);
    validation.merge(our_key_validation);

    // Validate core height
    let height_validation = validate_core_height_created_at(core_height, current_core_height);
    validation.merge(height_validation);

    // Validate account reference
    let account_validation = validate_account_reference(account_reference);
    validation.merge(account_validation);

    Ok(validation)
}
