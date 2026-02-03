//! Key Exchange Request data structures for YAPPR protocol
//!
//! This module handles parsing and validation of `dash-key:` URIs used for
//! web app key exchange (YAPPR - Yet Another Protocol for Platform Requests).

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::base58;
use dash_sdk::platform::Identifier;

/// Maximum label length in bytes (per spec)
const MAX_LABEL_LENGTH: usize = 64;

/// A parsed key exchange request from a `dash-key:` URI.
///
/// Web apps generate these URIs (displayed as QR codes or copyable text) to request
/// a deterministic login key from the wallet.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyExchangeRequest {
    /// Protocol version (must be 1 for current implementation)
    pub version: u8,
    /// App's ephemeral public key for ECDH (33 bytes compressed secp256k1)
    pub app_ephemeral_pub_key: [u8; 33],
    /// Target contract ID that the login key is for
    pub contract_id: Identifier,
    /// Key derivation index
    pub key_index: u32,
    /// Optional display label for the app (0-64 chars)
    pub label: Option<String>,
}

impl KeyExchangeRequest {
    /// Parse a `dash-key:` URI into a KeyExchangeRequest and network.
    ///
    /// URI Format: `dash-key:<base58_data>?n=<network>&v=<version>`
    ///
    /// The base58 data contains (per YAPPR spec Section 8.1):
    /// - Version (1 byte) - offset 0
    /// - App ephemeral public key (33 bytes) - offset 1-33
    /// - Contract ID (32 bytes) - offset 34-65
    /// - Key index (4 bytes, little-endian) - offset 66-69
    /// - Label length (1 byte) - offset 70
    /// - Label (0-64 bytes) - offset 71+
    ///
    /// Query parameters:
    /// - `n`: Network (mainnet, testnet, devnet) - required
    /// - `v`: Version (must be 1) - required, must match payload version
    ///
    /// # Returns
    /// A tuple of (KeyExchangeRequest, Network) on success, or an error string.
    pub fn from_uri(uri: &str) -> Result<(Self, Network), String> {
        // Check prefix
        if !uri.starts_with("dash-key:") {
            return Err("Invalid URI format - must start with 'dash-key:'".to_string());
        }

        // Split off the prefix
        let rest = &uri[9..]; // Skip "dash-key:"

        // Split into data and query parts
        let (data_part, query_part) = if let Some(pos) = rest.find('?') {
            (&rest[..pos], Some(&rest[pos + 1..]))
        } else {
            (rest, None)
        };

        // Parse query parameters
        let mut query_version: Option<u8> = None;
        let mut network = Network::Dash; // Default to mainnet

        if let Some(query) = query_part {
            for param in query.split('&') {
                let parts: Vec<&str> = param.split('=').collect();
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
                        network = match parts[1].to_lowercase().as_str() {
                            "mainnet" | "dash" | "m" => Network::Dash,
                            "testnet" | "t" => Network::Testnet,
                            "devnet" | "d" => Network::Devnet,
                            "regtest" | "r" => Network::Regtest,
                            _ => return Err(format!("Unknown network: {}", parts[1])),
                        };
                    }
                    _ => {} // Ignore unknown parameters
                }
            }
        }

        // Decode base58 data
        let data =
            base58::decode(data_part).map_err(|e| format!("Invalid base58 encoding: {}", e))?;

        // Minimum data length: 1 (version) + 33 (pubkey) + 32 (contract_id) + 4 (key_index) + 1 (label_len) = 71
        if data.len() < 71 {
            return Err(format!("Data too short: {} bytes (minimum 71)", data.len()));
        }

        // Parse version from payload (1 byte at offset 0)
        let payload_version = data[0];

        // Validate version
        if payload_version != 1 {
            return Err(format!(
                "Unsupported protocol version: {} (only version 1 is supported)",
                payload_version
            ));
        }

        // If query version is specified, it must match payload version
        if let Some(qv) = query_version
            && qv != payload_version
        {
            return Err(format!(
                "Version mismatch: query parameter v={} but payload version is {}",
                qv, payload_version
            ));
        }

        // Parse app ephemeral public key (33 bytes at offset 1-33)
        let app_ephemeral_pub_key: [u8; 33] = data[1..34]
            .try_into()
            .map_err(|_| "Failed to parse ephemeral public key")?;

        // Validate the public key format (must start with 0x02 or 0x03 for compressed)
        if app_ephemeral_pub_key[0] != 0x02 && app_ephemeral_pub_key[0] != 0x03 {
            return Err("Invalid ephemeral public key format (must be compressed)".to_string());
        }

        // Parse contract ID (32 bytes at offset 34-65)
        let contract_id_bytes: [u8; 32] = data[34..66]
            .try_into()
            .map_err(|_| "Failed to parse contract ID")?;
        let contract_id = Identifier::from_bytes(&contract_id_bytes)
            .map_err(|e| format!("Invalid contract ID: {}", e))?;

        // Parse key index (4 bytes, little-endian at offset 66-69)
        let key_index = u32::from_le_bytes(
            data[66..70]
                .try_into()
                .map_err(|_| "Failed to parse key index")?,
        );

        // Parse label length (1 byte at offset 70)
        let label_len = data[70] as usize;

        // Validate label length
        if label_len > MAX_LABEL_LENGTH {
            return Err(format!(
                "Label too long: {} bytes (maximum {})",
                label_len, MAX_LABEL_LENGTH
            ));
        }

        // Validate total length matches expected
        let expected_len = 71 + label_len;
        if data.len() != expected_len {
            return Err(format!(
                "Invalid data length: {} bytes (expected {})",
                data.len(),
                expected_len
            ));
        }

        // Parse label if present (offset 71+)
        let label = if label_len > 0 {
            let label_bytes = &data[71..71 + label_len];
            Some(String::from_utf8(label_bytes.to_vec()).map_err(|_| "Invalid UTF-8 in label")?)
        } else {
            None
        };

        Ok((
            Self {
                version: payload_version,
                app_ephemeral_pub_key,
                contract_id,
                key_index,
                label,
            },
            network,
        ))
    }

    /// Get the display name for this request (label or "Unknown App")
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or("Unknown App")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_uri(
        contract_id: &[u8; 32],
        key_index: u32,
        pubkey: &[u8; 33],
        label: Option<&str>,
        network: Option<&str>,
        version: Option<u8>,
    ) -> String {
        // Build data per YAPPR spec Section 8.1:
        // - Version (1 byte) - offset 0
        // - App ephemeral public key (33 bytes) - offset 1-33
        // - Contract ID (32 bytes) - offset 34-65
        // - Key index (4 bytes, little-endian) - offset 66-69
        // - Label length (1 byte) - offset 70
        // - Label (0-64 bytes) - offset 71+
        let mut data = Vec::new();
        data.push(version.unwrap_or(1)); // Version byte
        data.extend_from_slice(pubkey); // 33 bytes
        data.extend_from_slice(contract_id); // 32 bytes
        data.extend_from_slice(&key_index.to_le_bytes()); // 4 bytes, little-endian

        if let Some(l) = label {
            data.push(l.len() as u8);
            data.extend_from_slice(l.as_bytes());
        } else {
            data.push(0);
        }

        let encoded = base58::encode_slice(&data);
        let mut uri = format!("dash-key:{}", encoded);

        let mut params = Vec::new();
        if let Some(n) = network {
            params.push(format!("n={}", n));
        }
        if let Some(v) = version {
            params.push(format!("v={}", v));
        }

        if !params.is_empty() {
            uri.push('?');
            uri.push_str(&params.join("&"));
        }

        uri
    }

    #[test]
    fn test_parse_valid_uri_no_label() {
        let contract_id = [0x11u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x02; // Compressed pubkey prefix
            p
        };

        let uri = create_test_uri(&contract_id, 42, &pubkey, None, Some("testnet"), Some(1));

        let (request, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");

        assert_eq!(request.version, 1);
        assert_eq!(request.key_index, 42);
        assert_eq!(request.app_ephemeral_pub_key, pubkey);
        assert_eq!(request.label, None);
        assert_eq!(network, Network::Testnet);
    }

    #[test]
    fn test_parse_valid_uri_with_label() {
        let contract_id = [0x22u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x03;
            p
        };

        let uri = create_test_uri(
            &contract_id,
            100,
            &pubkey,
            Some("My Cool App"),
            Some("mainnet"),
            Some(1),
        );

        let (request, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");

        assert_eq!(request.version, 1);
        assert_eq!(request.key_index, 100);
        assert_eq!(request.label, Some("My Cool App".to_string()));
        assert_eq!(request.display_name(), "My Cool App");
        assert_eq!(network, Network::Dash);
    }

    #[test]
    fn test_parse_default_network_and_version() {
        let contract_id = [0x33u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x02;
            p
        };

        // No query parameters - should default to mainnet and version 1
        let uri = create_test_uri(&contract_id, 0, &pubkey, None, None, None);

        let (request, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");

        assert_eq!(request.version, 1);
        assert_eq!(network, Network::Dash);
    }

    #[test]
    fn test_parse_short_network_codes() {
        let contract_id = [0x33u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x02;
            p
        };

        // Test "t" for testnet
        let uri = create_test_uri(&contract_id, 0, &pubkey, None, Some("t"), Some(1));
        let (_, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");
        assert_eq!(network, Network::Testnet);

        // Test "m" for mainnet
        let uri = create_test_uri(&contract_id, 0, &pubkey, None, Some("m"), Some(1));
        let (_, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");
        assert_eq!(network, Network::Dash);

        // Test "d" for devnet
        let uri = create_test_uri(&contract_id, 0, &pubkey, None, Some("d"), Some(1));
        let (_, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");
        assert_eq!(network, Network::Devnet);

        // Test "r" for regtest
        let uri = create_test_uri(&contract_id, 0, &pubkey, None, Some("r"), Some(1));
        let (_, network) = KeyExchangeRequest::from_uri(&uri).expect("Should parse");
        assert_eq!(network, Network::Regtest);
    }

    #[test]
    fn test_reject_unsupported_version() {
        let contract_id = [0x44u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x02;
            p
        };

        let uri = create_test_uri(&contract_id, 0, &pubkey, None, None, Some(2));

        let result = KeyExchangeRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported protocol version"));
    }

    #[test]
    fn test_reject_invalid_prefix() {
        let result = KeyExchangeRequest::from_uri("dash:?di=abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with 'dash-key:'"));
    }

    #[test]
    fn test_reject_invalid_pubkey_format() {
        let contract_id = [0x55u8; 32];
        let pubkey = [0x04u8; 33]; // Invalid - 0x04 is uncompressed prefix

        let uri = create_test_uri(&contract_id, 0, &pubkey, None, None, None);

        let result = KeyExchangeRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be compressed"));
    }

    #[test]
    fn test_reject_label_too_long() {
        let contract_id = [0x66u8; 32];
        let pubkey = {
            let mut p = [0u8; 33];
            p[0] = 0x02;
            p
        };

        // Create URI with 65-byte label (exceeds 64 byte limit)
        let long_label = "x".repeat(65);
        let uri = create_test_uri(&contract_id, 0, &pubkey, Some(&long_label), None, None);

        let result = KeyExchangeRequest::from_uri(&uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Label too long"));
    }

    #[test]
    fn test_display_name_with_label() {
        let request = KeyExchangeRequest {
            version: 1,
            app_ephemeral_pub_key: [0x02; 33],
            contract_id: Identifier::from_bytes(&[0u8; 32]).unwrap(),
            key_index: 0,
            label: Some("Test App".to_string()),
        };

        assert_eq!(request.display_name(), "Test App");
    }

    #[test]
    fn test_display_name_without_label() {
        let request = KeyExchangeRequest {
            version: 1,
            app_ephemeral_pub_key: [0x02; 33],
            contract_id: Identifier::from_bytes(&[0u8; 32]).unwrap(),
            key_index: 0,
            label: None,
        };

        assert_eq!(request.display_name(), "Unknown App");
    }
    #[test]
    fn test_parse_real_yappr_uris() {
        // Real URIs from Yappr app - both should parse to the same contract ID
        let uri1 = "dash-key:KW76jFtvfVEtLAhZAhfAXMjS9kQHFTiuHg4v2yTyTEXurNUvDz14brC1YFKW72u5R1UgHQkfGS7GWdEYqRCVdk4XCEuieHq8xUDQBwmqwFxdyKGdPRT?n=t&v=1";
        let uri2 = "dash-key:KTeX5FvM4rd8jp3sSfZESA9TKokwH1nsRuZZNFno8j7aFitsuordJRdVu5NmieVd7YL77XMYrjm4GzGQS36fTfCK4rR5XiVVxnTTMM7EWqSR8VbybCD?n=t&v=1";

        let (request1, network1) = KeyExchangeRequest::from_uri(uri1).expect("Should parse URI 1");
        let (request2, network2) = KeyExchangeRequest::from_uri(uri2).expect("Should parse URI 2");

        // Both should be testnet
        assert_eq!(network1, Network::Testnet);
        assert_eq!(network2, Network::Testnet);

        // Both should have version 1
        assert_eq!(request1.version, 1);
        assert_eq!(request2.version, 1);

        // Contract IDs should be the same (same app)
        assert_eq!(
            request1.contract_id, request2.contract_id,
            "Contract IDs should match for the same app"
        );

        // Key indices should be 0
        assert_eq!(request1.key_index, 0);
        assert_eq!(request2.key_index, 0);

        // Labels should be "Login to Yappr"
        assert_eq!(request1.label, Some("Login to Yappr".to_string()));
        assert_eq!(request2.label, Some("Login to Yappr".to_string()));

        // Ephemeral public keys should be different (fresh for each request)
        assert_ne!(
            request1.app_ephemeral_pub_key, request2.app_ephemeral_pub_key,
            "Ephemeral keys should differ between requests"
        );

        // Both ephemeral keys should start with 0x02 or 0x03 (compressed pubkey)
        assert!(
            request1.app_ephemeral_pub_key[0] == 0x02 || request1.app_ephemeral_pub_key[0] == 0x03
        );
        assert!(
            request2.app_ephemeral_pub_key[0] == 0x02 || request2.app_ephemeral_pub_key[0] == 0x03
        );
    }
}
