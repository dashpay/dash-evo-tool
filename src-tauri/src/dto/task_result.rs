//! Task result DTOs for typed event payloads.
//!
//! These types replace the untyped `(String, Option<serde_json::Value>)` pairs
//! that were previously used in `TaskResultEvent`. The frontend receives a
//! discriminated union (`TaskResultPayloadDto`) with fully typed fields,
//! eliminating manual casting and magic-string comparisons.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Domain discriminant for routing error events to the correct store.
///
/// When a `TaskErrorEvent` is emitted, the `domain` field tells the frontend
/// which store should handle the error, preventing a single error from
/// clearing loading state in all stores simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskDomain {
    Identity,
    Wallet,
    Contract,
    Document,
    Token,
    Contest,
    DashPay,
    Platform,
    MnList,
    Core,
    System,
    GroveStark,
    General,
}

/// Typed task result payload — an internally-tagged discriminated union.
///
/// Each variant carries exactly the data the frontend needs. The `type` tag
/// is used by TypeScript to narrow the union.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskResultPayloadDto {
    /// No meaningful result data.
    #[serde(rename = "none")]
    None,
    /// Signal to refresh/reload data from the database.
    #[serde(rename = "refresh")]
    Refresh,
    /// A progress or informational message.
    #[serde(rename = "message")]
    Message {
        text: String,
    },
    /// A state transition was broadcast successfully.
    #[serde(rename = "broadcastedStateTransition")]
    BroadcastedStateTransition,
    /// An identity operation completed.
    #[serde(rename = "identityCompleted")]
    IdentityCompleted {
        /// The affected identity ID (hex), if known.
        #[serde(rename = "identityId")]
        identity_id: Option<String>,
    },
    /// A wallet operation completed.
    #[serde(rename = "walletCompleted")]
    WalletCompleted,
    /// A new receive address was generated for a wallet.
    #[serde(rename = "walletGeneratedAddress")]
    WalletGeneratedAddress {
        address: String,
    },
    /// A contract operation completed.
    #[serde(rename = "contractCompleted")]
    ContractCompleted,
    /// A page of documents was fetched.
    #[serde(rename = "documentPage")]
    DocumentPage {
        documents: Vec<serde_json::Value>,
        #[serde(rename = "hasMore")]
        has_more: bool,
    },
    /// A document operation completed (non-page result).
    #[serde(rename = "documentCompleted")]
    DocumentCompleted,
    /// A token operation completed.
    #[serde(rename = "tokenCompleted")]
    TokenCompleted,
    /// Token balances were fetched from Platform and saved to the local DB.
    /// The frontend should call `token_get_my_balances` to read the updated data.
    #[serde(rename = "tokenBalancesLoaded")]
    TokenBalancesLoaded,
    /// Frozen identity IDs for a given token.
    #[serde(rename = "tokenFrozenIdentities")]
    TokenFrozenIdentities {
        #[serde(rename = "identityIds")]
        identity_ids: Vec<String>,
    },
    /// A contest/DPNS operation completed.
    #[serde(rename = "contestCompleted")]
    ContestCompleted,
    /// A DashPay operation completed (non-search result).
    #[serde(rename = "dashPayCompleted")]
    DashPayCompleted,
    /// DashPay profile search results (transient, not stored in DB).
    #[serde(rename = "dashPayProfileSearchResults")]
    DashPayProfileSearchResults {
        results: Vec<ProfileSearchResultDto>,
    },
    /// Platform info as pre-formatted text.
    #[serde(rename = "platformText")]
    PlatformText {
        title: String,
        data: String,
    },
    /// Platform address balance query result.
    #[serde(rename = "platformAddressBalance")]
    PlatformAddressBalance {
        address: String,
        balance: u64,
        nonce: u32,
    },
    /// A single masternode list diff.
    #[serde(rename = "mnListFetchedDiff")]
    MnListFetchedDiff {
        #[serde(rename = "baseHeight")]
        base_height: u32,
        height: u32,
        diff: serde_json::Value,
    },
    /// Multiple masternode list diffs.
    #[serde(rename = "mnListFetchedDiffs")]
    MnListFetchedDiffs {
        diffs: Vec<serde_json::Value>,
    },
    /// Quorum rotation info.
    #[serde(rename = "mnListFetchedQrInfo")]
    MnListFetchedQrInfo {
        #[serde(rename = "qrInfo")]
        qr_info: serde_json::Value,
    },
    /// Chain lock signatures.
    #[serde(rename = "mnListChainLockSigs")]
    MnListChainLockSigs {
        entries: Vec<TaskChainLockSigDto>,
    },
    /// A Core operation completed.
    #[serde(rename = "coreCompleted")]
    CoreCompleted,
    /// A System operation completed.
    #[serde(rename = "systemCompleted")]
    SystemCompleted,
    /// A GroveSTARK operation completed.
    #[serde(rename = "groveStarkCompleted")]
    GroveStarkCompleted,

    // ── Transient token results (not stored in DB) ──────────────────

    /// Token pricing schedule for a specific token (transient query result).
    #[serde(rename = "tokenPricing")]
    TokenPricing {
        /// Token ID (hex).
        #[serde(rename = "tokenId")]
        token_id: String,
        /// Pricing schedule as JSON, or null if no pricing set.
        prices: Option<serde_json::Value>,
    },

    /// Estimated distribution rewards for a token (transient query result).
    #[serde(rename = "tokenRewardEstimate")]
    TokenRewardEstimate {
        /// Token ID (hex).
        #[serde(rename = "tokenId")]
        token_id: String,
        /// Identity ID (hex).
        #[serde(rename = "identityId")]
        identity_id: String,
        /// Total estimated reward amount (string for u128).
        amount: String,
        /// Detailed explanation text.
        explanation: String,
    },

    /// Token search results by keyword (transient query result).
    #[serde(rename = "tokenSearchResults")]
    TokenSearchResults {
        /// Search result entries.
        results: Vec<TokenSearchResultDto>,
        /// Whether more results are available (pagination).
        #[serde(rename = "hasMore")]
        has_more: bool,
    },

    /// Token not found on Platform.
    #[serde(rename = "tokenNotFound")]
    TokenNotFound,

    // ── Transient contract results (not stored in DB) ───────────────

    /// Contract(s) with token descriptions (transient query result).
    #[serde(rename = "contractWithDescriptions")]
    ContractWithDescriptions {
        /// Contracts with their token info.
        contracts: Vec<ContractWithTokensDto>,
    },
}

/// A profile search result from DashPay.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSearchResultDto {
    pub identity_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub public_message: Option<String>,
    pub avatar_url: Option<String>,
}

/// A token search result entry (keyword search).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenSearchResultDto {
    /// Contract ID (hex).
    pub contract_id: String,
    /// Description text.
    pub description: String,
}

/// A token within a contract (for contract-with-descriptions results).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContractTokenInfoDto {
    /// Token ID (hex).
    pub token_id: String,
    /// Token name.
    pub name: String,
    /// Token description (if available).
    pub description: Option<String>,
    /// Token position within the contract.
    pub token_position: u16,
    /// Token configuration as JSON (for schema viewing).
    pub configuration_json: Option<serde_json::Value>,
}

/// A contract with its tokens (for contract-with-descriptions results).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContractWithTokensDto {
    /// Contract ID (hex).
    pub contract_id: String,
    /// Contract description (if available).
    pub description: Option<String>,
    /// Tokens in this contract.
    pub tokens: Vec<ContractTokenInfoDto>,
}

/// A chain lock signature entry.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskChainLockSigDto {
    pub height: u32,
    pub block_hash: String,
    pub signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_domain_serializes_camel_case() {
        let json = serde_json::to_string(&TaskDomain::GroveStark).unwrap();
        assert_eq!(json, "\"groveStark\"");

        let json = serde_json::to_string(&TaskDomain::DashPay).unwrap();
        assert_eq!(json, "\"dashPay\"");

        let json = serde_json::to_string(&TaskDomain::MnList).unwrap();
        assert_eq!(json, "\"mnList\"");
    }

    #[test]
    fn payload_none_serializes_with_type_tag() {
        let payload = TaskResultPayloadDto::None;
        let json = serde_json::to_string(&payload).unwrap();
        assert_eq!(json, r#"{"type":"none"}"#);
    }

    #[test]
    fn payload_message_serializes() {
        let payload = TaskResultPayloadDto::Message {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""type":"message""#));
        assert!(json.contains(r#""text":"hello""#));
    }

    #[test]
    fn payload_identity_completed_serializes() {
        let payload = TaskResultPayloadDto::IdentityCompleted {
            identity_id: Some("abc123".into()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""type":"identityCompleted""#));
        assert!(json.contains(r#""identityId":"abc123""#));
    }

    #[test]
    fn payload_document_page_serializes() {
        let payload = TaskResultPayloadDto::DocumentPage {
            documents: vec![serde_json::json!({"id": "doc1"})],
            has_more: true,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""type":"documentPage""#));
        assert!(json.contains(r#""hasMore":true"#));
    }

    #[test]
    fn payload_platform_address_balance_serializes() {
        let payload = TaskResultPayloadDto::PlatformAddressBalance {
            address: "yAddr123".into(),
            balance: 100_000_000,
            nonce: 5,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""type":"platformAddressBalance""#));
        assert!(json.contains(r#""address":"yAddr123""#));
        assert!(json.contains(r#""balance":100000000"#));
        assert!(json.contains(r#""nonce":5"#));
    }

    #[test]
    fn profile_search_result_serializes_camel_case() {
        let result = ProfileSearchResultDto {
            identity_id: "id1".into(),
            username: "alice".into(),
            display_name: Some("Alice".into()),
            public_message: None,
            avatar_url: Some("https://example.com/avatar.png".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""identityId":"id1""#));
        assert!(json.contains(r#""displayName":"Alice""#));
        assert!(json.contains(r#""publicMessage":null"#));
        assert!(json.contains(r#""avatarUrl":"https://example.com/avatar.png""#));
    }

    #[test]
    fn chain_lock_sig_entry_serializes() {
        let entry = TaskChainLockSigDto {
            height: 12345,
            block_hash: "00aabb".into(),
            signature: Some("sig123".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""blockHash":"00aabb""#));
        assert!(json.contains(r#""signature":"sig123""#));
    }

    #[test]
    fn payload_token_frozen_identities_serializes() {
        let payload = TaskResultPayloadDto::TokenFrozenIdentities {
            identity_ids: vec!["aabb1122".into(), "ccdd3344".into()],
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""type":"tokenFrozenIdentities""#));
        assert!(json.contains(r#""identityIds":["aabb1122","ccdd3344"]"#));
    }

    #[test]
    fn payload_mnlist_variants_serialize() {
        let diff = TaskResultPayloadDto::MnListFetchedDiff {
            base_height: 100,
            height: 200,
            diff: serde_json::json!({}),
        };
        let json = serde_json::to_string(&diff).unwrap();
        assert!(json.contains(r#""type":"mnListFetchedDiff""#));
        assert!(json.contains(r#""baseHeight":100"#));

        let diffs = TaskResultPayloadDto::MnListFetchedDiffs {
            diffs: vec![serde_json::json!({"a": 1})],
        };
        let json = serde_json::to_string(&diffs).unwrap();
        assert!(json.contains(r#""type":"mnListFetchedDiffs""#));

        let qr = TaskResultPayloadDto::MnListFetchedQrInfo {
            qr_info: serde_json::json!({"info": true}),
        };
        let json = serde_json::to_string(&qr).unwrap();
        assert!(json.contains(r#""type":"mnListFetchedQrInfo""#));

        let sigs = TaskResultPayloadDto::MnListChainLockSigs {
            entries: vec![TaskChainLockSigDto {
                height: 1,
                block_hash: "hash".into(),
                signature: None,
            }],
        };
        let json = serde_json::to_string(&sigs).unwrap();
        assert!(json.contains(r#""type":"mnListChainLockSigs""#));
    }
}
