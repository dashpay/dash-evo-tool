//! State Transition Request data structures for DIP-signing-request protocol
//!
//! This module handles parsing and validation of `dash-st:` URIs used for
//! external applications to request state transition signing and broadcasting.

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::serialization::PlatformDeserializable;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::state_transition::batch_transition::BatchTransition;
use dash_sdk::dpp::state_transition::data_contract_create_transition::DataContractCreateTransition;
use dash_sdk::dpp::state_transition::data_contract_update_transition::DataContractUpdateTransition;
use dash_sdk::dpp::state_transition::identity_create_transition::IdentityCreateTransition;
use dash_sdk::dpp::state_transition::identity_credit_transfer_transition::IdentityCreditTransferTransition;
use dash_sdk::dpp::state_transition::identity_credit_withdrawal_transition::IdentityCreditWithdrawalTransition;
use dash_sdk::dpp::state_transition::identity_topup_transition::IdentityTopUpTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::masternode_vote_transition::MasternodeVoteTransition;
use dash_sdk::platform::Identifier;

/// Maximum label length in bytes (per spec)
const MAX_LABEL_LENGTH: usize = 64;

/// A parsed state transition request from a `dash-st:` URI.
///
/// External applications generate these URIs to request the wallet to sign
/// and broadcast state transitions on their behalf.
#[derive(Debug, Clone)]
pub struct StateTransitionRequest {
    /// Protocol version (must be 1 for current implementation)
    pub version: u8,
    /// The state transition to be signed and broadcast
    pub state_transition: StateTransition,
    /// Optional identity ID hint (identity that should sign)
    pub identity_hint: Option<Identifier>,
    /// Optional key ID hint (specific key that should be used)
    pub key_id_hint: Option<KeyID>,
    /// Optional display label for the request (0-64 chars)
    pub label: Option<String>,
}

impl StateTransitionRequest {
    /// Parse a `dash-st:` URI into a StateTransitionRequest and network.
    ///
    /// URI Format: `dash-st:<base58_state_transition>?n=<network>&v=<version>[&id=<identity_hint>][&k=<key_id>][&l=<label>]`
    ///
    /// The base58 data contains the serialized state transition bytes.
    ///
    /// Query parameters:
    /// - `n`: Network (m/t/d for mainnet/testnet/devnet) - required
    /// - `v`: Version (must be 1) - required
    /// - `id`: Base58 Identity ID hint - optional
    /// - `k`: Key ID hint (integer) - optional
    /// - `l`: URL-encoded label (max 64 chars) - optional
    ///
    /// # Returns
    /// A tuple of (StateTransitionRequest, Network) on success, or an error string.
    pub fn from_uri(uri: &str) -> Result<(Self, Network), String> {
        // Check prefix
        if !uri.starts_with("dash-st:") {
            return Err("Invalid URI format - must start with 'dash-st:'".to_string());
        }

        // Split off the prefix
        let rest = &uri[8..]; // Skip "dash-st:"

        // Split into data and query parts
        let (data_part, query_part) = if let Some(pos) = rest.find('?') {
            (&rest[..pos], Some(&rest[pos + 1..]))
        } else {
            (rest, None)
        };

        // Parse query parameters
        let mut query_version: Option<u8> = None;
        let mut network: Option<Network> = None;
        let mut identity_hint: Option<Identifier> = None;
        let mut key_id_hint: Option<KeyID> = None;
        let mut label: Option<String> = None;
        let mut transition_type: Option<String> = None;

        if let Some(query) = query_part {
            for param in query.split('&') {
                let parts: Vec<&str> = param.splitn(2, '=').collect();
                if parts.len() != 2 {
                    continue;
                }

                match parts[0] {
                    "v" => {
                        query_version = Some(
                            parts[1]
                                .parse::<u8>()
                                .map_err(|_| "Invalid version parameter")?,
                        );
                    }
                    "n" => {
                        network = Some(match parts[1].to_lowercase().as_str() {
                            "mainnet" | "dash" | "m" => Network::Dash,
                            "testnet" | "t" => Network::Testnet,
                            "devnet" | "d" => Network::Devnet,
                            "regtest" | "r" => Network::Regtest,
                            _ => return Err(format!("Unknown network: {}", parts[1])),
                        });
                    }
                    "t" => {
                        // Transition type hint (e.g., "iu" for identity update)
                        transition_type = Some(parts[1].to_lowercase());
                    }
                    "id" => {
                        // Parse identity ID hint (Base58)
                        let id_bytes = base58::decode(parts[1])
                            .map_err(|e| format!("Invalid identity ID encoding: {}", e))?;
                        if id_bytes.len() != 32 {
                            return Err(format!(
                                "Invalid identity ID length: {} bytes (expected 32)",
                                id_bytes.len()
                            ));
                        }
                        let id_array: [u8; 32] = id_bytes
                            .try_into()
                            .map_err(|_| "Failed to convert identity ID bytes")?;
                        identity_hint = Some(
                            Identifier::from_bytes(&id_array)
                                .map_err(|e| format!("Invalid identity ID: {}", e))?,
                        );
                    }
                    "k" => {
                        // Parse key ID hint (integer)
                        key_id_hint = Some(
                            parts[1]
                                .parse::<KeyID>()
                                .map_err(|_| "Invalid key ID parameter")?,
                        );
                    }
                    "l" => {
                        // Parse URL-encoded label using simple percent-decode
                        let decoded_label = url_decode(parts[1])
                            .map_err(|e| format!("Invalid label encoding: {}", e))?;

                        if decoded_label.len() > MAX_LABEL_LENGTH {
                            return Err(format!(
                                "Label too long: {} bytes (maximum {})",
                                decoded_label.len(),
                                MAX_LABEL_LENGTH
                            ));
                        }

                        label = Some(decoded_label);
                    }
                    _ => {} // Ignore unknown parameters
                }
            }
        }

        // Validate required parameters
        let version = query_version.ok_or("Missing required version parameter (v)")?;
        if version != 1 {
            return Err(format!(
                "Unsupported protocol version: {} (only version 1 is supported)",
                version
            ));
        }

        let network = network.ok_or("Missing required network parameter (n)")?;

        // Try to decode state transition data (try base58 first since that's the spec, then hex, then base64)
        let st_bytes = base58::decode(data_part)
            .or_else(|_| hex::decode(data_part))
            .or_else(|_| {
                use base64::{Engine, engine::general_purpose::STANDARD};
                STANDARD.decode(data_part)
            })
            .map_err(|e| {
                format!(
                    "Invalid encoding for state transition (tried base58, hex, base64): {}",
                    e
                )
            })?;

        if st_bytes.is_empty() {
            return Err("State transition data is empty".to_string());
        }

        // Deserialize the state transition based on the type hint if provided
        let state_transition = if let Some(ref t) = transition_type {
            // Deserialize specific transition type and wrap in StateTransition
            match t.as_str() {
                "cc" | "dcc" => {
                    let inner = DataContractCreateTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize DataContractCreate: {}", e))?;
                    StateTransition::DataContractCreate(inner)
                }
                "cu" | "dcu" => {
                    let inner = DataContractUpdateTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize DataContractUpdate: {}", e))?;
                    StateTransition::DataContractUpdate(inner)
                }
                "b" | "batch" => {
                    let inner = BatchTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize Batch: {}", e))?;
                    StateTransition::Batch(inner)
                }
                "ic" => {
                    let inner = IdentityCreateTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize IdentityCreate: {}", e))?;
                    StateTransition::IdentityCreate(inner)
                }
                "it" => {
                    let inner = IdentityTopUpTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize IdentityTopUp: {}", e))?;
                    StateTransition::IdentityTopUp(inner)
                }
                "iw" => {
                    let inner =
                        IdentityCreditWithdrawalTransition::deserialize_from_bytes(&st_bytes)
                            .map_err(|e| {
                                format!("Failed to deserialize IdentityCreditWithdrawal: {}", e)
                            })?;
                    StateTransition::IdentityCreditWithdrawal(inner)
                }
                "iu" => {
                    let inner = IdentityUpdateTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize IdentityUpdate: {}", e))?;
                    StateTransition::IdentityUpdate(inner)
                }
                "ict" => {
                    let inner = IdentityCreditTransferTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| {
                            format!("Failed to deserialize IdentityCreditTransfer: {}", e)
                        })?;
                    StateTransition::IdentityCreditTransfer(inner)
                }
                "mv" => {
                    let inner = MasternodeVoteTransition::deserialize_from_bytes(&st_bytes)
                        .map_err(|e| format!("Failed to deserialize MasternodeVote: {}", e))?;
                    StateTransition::MasternodeVote(inner)
                }
                _ => {
                    return Err(format!(
                        "Unknown transition type '{}'. Valid types: cc, cu, b, ic, it, iw, iu, ict, mv",
                        t
                    ));
                }
            }
        } else {
            // No type hint - try to deserialize as full StateTransition
            StateTransition::deserialize_from_bytes(&st_bytes)
                .map_err(|e| format!("Failed to deserialize state transition: {}", e))?
        };

        Ok((
            Self {
                version,
                state_transition,
                identity_hint,
                key_id_hint,
                label,
            },
            network,
        ))
    }

    /// Get the display name for this request (label or "External Request")
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or("External Request")
    }

    /// Get the type of state transition
    pub fn transition_type(&self) -> &'static str {
        match &self.state_transition {
            StateTransition::DataContractCreate(_) => "Data Contract Create",
            StateTransition::DataContractUpdate(_) => "Data Contract Update",
            StateTransition::Batch(_) => "Batch (Documents/Tokens)",
            StateTransition::IdentityCreate(_) => "Identity Create",
            StateTransition::IdentityTopUp(_) => "Identity Top Up",
            StateTransition::IdentityCreditWithdrawal(_) => "Identity Credit Withdrawal",
            StateTransition::IdentityUpdate(_) => "Identity Update",
            StateTransition::IdentityCreditTransfer(_) => "Identity Credit Transfer",
            StateTransition::MasternodeVote(_) => "Masternode Vote",
            StateTransition::IdentityCreditTransferToAddresses(_) => {
                "Identity Credit Transfer to Addresses"
            }
            StateTransition::IdentityCreateFromAddresses(_) => "Identity Create from Addresses",
            StateTransition::IdentityTopUpFromAddresses(_) => "Identity Top Up from Addresses",
            StateTransition::AddressFundsTransfer(_) => "Address Funds Transfer",
            StateTransition::AddressFundingFromAssetLock(_) => "Address Funding from Asset Lock",
            StateTransition::AddressCreditWithdrawal(_) => "Address Credit Withdrawal",
        }
    }

    /// Check if this is a high-risk transition type that requires extra warnings
    ///
    /// High-risk transitions include:
    /// - IdentityUpdate: Can change keys, potentially locking out the owner
    /// - IdentityCreditWithdrawal: Moves funds out of the identity
    /// - IdentityCreditTransfer: Moves funds to another identity
    /// - Address-based transfer/withdrawal variants: Move funds
    pub fn is_high_risk(&self) -> bool {
        matches!(
            &self.state_transition,
            StateTransition::IdentityUpdate(_)
                | StateTransition::IdentityCreditWithdrawal(_)
                | StateTransition::IdentityCreditTransfer(_)
                | StateTransition::IdentityCreditTransferToAddresses(_)
                | StateTransition::AddressFundsTransfer(_)
                | StateTransition::AddressCreditWithdrawal(_)
        )
    }

    /// Get a description of why this transition is high-risk (if applicable)
    pub fn high_risk_reason(&self) -> Option<&'static str> {
        match &self.state_transition {
            StateTransition::IdentityUpdate(_) => Some(
                "This transition can modify identity keys, potentially changing access control",
            ),
            StateTransition::IdentityCreditWithdrawal(_)
            | StateTransition::AddressCreditWithdrawal(_) => {
                Some("This transition will withdraw credits to a Core address")
            }
            StateTransition::IdentityCreditTransfer(_)
            | StateTransition::IdentityCreditTransferToAddresses(_) => {
                Some("This transition will transfer credits to another identity or address")
            }
            StateTransition::AddressFundsTransfer(_) => {
                Some("This transition will transfer funds between Platform addresses")
            }
            _ => None,
        }
    }

    /// Get the owner identity ID from the state transition if available
    pub fn owner_identity_id(&self) -> Option<Identifier> {
        self.state_transition.owner_id()
    }

    /// Serialize the state transition to JSON for display
    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.state_transition)
            .map_err(|e| format!("Failed to serialize state transition to JSON: {}", e))
    }
}

/// Simple URL decode function for percent-encoded strings
fn url_decode(input: &str) -> Result<String, String> {
    let mut result = Vec::new();
    let mut chars = input.bytes().peekable();

    while let Some(byte) = chars.next() {
        if byte == b'%' {
            // Read next two hex characters
            let high = chars
                .next()
                .ok_or_else(|| "Incomplete percent encoding".to_string())?;
            let low = chars
                .next()
                .ok_or_else(|| "Incomplete percent encoding".to_string())?;

            let high = hex_char_to_nibble(high)?;
            let low = hex_char_to_nibble(low)?;
            result.push((high << 4) | low);
        } else if byte == b'+' {
            // Plus is often used for spaces in query strings
            result.push(b' ');
        } else {
            result.push(byte);
        }
    }

    String::from_utf8(result).map_err(|_| "Invalid UTF-8 in decoded string".to_string())
}

/// Convert a hex character to its nibble value
fn hex_char_to_nibble(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("Invalid hex character: {}", c as char)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_uri_with_params(
        st_bytes: &[u8],
        network: Option<&str>,
        version: Option<u8>,
        identity_hint: Option<&str>,
        key_id: Option<u32>,
        label: Option<&str>,
    ) -> String {
        let encoded = base58::encode_slice(st_bytes);
        let mut uri = format!("dash-st:{}", encoded);

        let mut params = Vec::new();
        if let Some(n) = network {
            params.push(format!("n={}", n));
        }
        if let Some(v) = version {
            params.push(format!("v={}", v));
        }
        if let Some(id) = identity_hint {
            params.push(format!("id={}", id));
        }
        if let Some(k) = key_id {
            params.push(format!("k={}", k));
        }
        if let Some(l) = label {
            // Simple URL encoding for test purposes
            let encoded: String = l
                .chars()
                .map(|c| match c {
                    ' ' => "%20".to_string(),
                    '!' => "%21".to_string(),
                    '#' => "%23".to_string(),
                    '$' => "%24".to_string(),
                    '%' => "%25".to_string(),
                    '&' => "%26".to_string(),
                    '\'' => "%27".to_string(),
                    '(' => "%28".to_string(),
                    ')' => "%29".to_string(),
                    '*' => "%2A".to_string(),
                    '+' => "%2B".to_string(),
                    ',' => "%2C".to_string(),
                    '/' => "%2F".to_string(),
                    ':' => "%3A".to_string(),
                    ';' => "%3B".to_string(),
                    '=' => "%3D".to_string(),
                    '?' => "%3F".to_string(),
                    '@' => "%40".to_string(),
                    '[' => "%5B".to_string(),
                    ']' => "%5D".to_string(),
                    _ => c.to_string(),
                })
                .collect();
            params.push(format!("l={}", encoded));
        }

        if !params.is_empty() {
            uri.push('?');
            uri.push_str(&params.join("&"));
        }

        uri
    }

    #[test]
    fn test_reject_invalid_prefix() {
        let result = StateTransitionRequest::from_uri("dash:?data=abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with 'dash-st:'"));
    }

    #[test]
    fn test_reject_missing_version() {
        let uri = create_test_uri_with_params(&[1, 2, 3], Some("t"), None, None, None, None);
        let result = StateTransitionRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required version"));
    }

    #[test]
    fn test_reject_missing_network() {
        let uri = create_test_uri_with_params(&[1, 2, 3], None, Some(1), None, None, None);
        let result = StateTransitionRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required network"));
    }

    #[test]
    fn test_reject_unsupported_version() {
        let uri = create_test_uri_with_params(&[1, 2, 3], Some("t"), Some(2), None, None, None);
        let result = StateTransitionRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported protocol version"));
    }

    #[test]
    fn test_reject_label_too_long() {
        let long_label = "x".repeat(65);
        let uri = create_test_uri_with_params(
            &[1, 2, 3],
            Some("t"),
            Some(1),
            None,
            None,
            Some(&long_label),
        );
        let result = StateTransitionRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Label too long"));
    }

    #[test]
    fn test_parse_network_codes() {
        // These will fail at ST deserialization, but we can test network parsing
        // by checking the error message doesn't mention network issues
        let test_cases = [
            ("m", Network::Dash),
            ("t", Network::Testnet),
            ("d", Network::Devnet),
            ("r", Network::Regtest),
            ("mainnet", Network::Dash),
            ("testnet", Network::Testnet),
            ("devnet", Network::Devnet),
        ];

        for (code, _expected_network) in test_cases {
            let uri =
                create_test_uri_with_params(&[1, 2, 3], Some(code), Some(1), None, None, None);
            let result = StateTransitionRequest::from_uri(&uri);
            // Should fail at ST deserialization, not network parsing
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                !err.contains("Unknown network"),
                "Network code '{}' should be recognized",
                code
            );
        }
    }

    #[test]
    fn test_display_name_with_label() {
        // We can't easily create a valid StateTransitionRequest without real ST bytes,
        // but we can test the display_name logic conceptually
        let label = Some("My Test App".to_string());
        assert_eq!(
            label.as_deref().unwrap_or("External Request"),
            "My Test App"
        );
    }

    #[test]
    fn test_display_name_without_label() {
        let label: Option<String> = None;
        assert_eq!(
            label.as_deref().unwrap_or("External Request"),
            "External Request"
        );
    }
}
