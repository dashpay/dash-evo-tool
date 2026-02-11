//! Proof log Tauri IPC commands.
//!
//! Provides paginated, sorted access to the proof log stored in the local
//! database. The proof log records every GroveDB proof received from
//! Platform, along with the request type, block height, timing, and any
//! verification errors.

use crate::state::AppState;

use dash_evo_tool::model::proof_log_item::{ProofLogItem, RequestType};

use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Serializable representation of a `RequestType` enum variant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RequestTypeDto {
    BroadcastStateTransition,
    GetIdentity,
    GetIdentityKeys,
    GetIdentitiesContractKeys,
    GetIdentityNonce,
    GetIdentityContractNonce,
    GetIdentityBalance,
    GetIdentitiesBalances,
    GetIdentityBalanceAndRevision,
    GetEvonodesProposedEpochBlocksByIds,
    GetEvonodesProposedEpochBlocksByRange,
    GetProofs,
    GetDataContract,
    GetDataContractHistory,
    GetDataContracts,
    GetDocuments,
    GetIdentityByPublicKeyHash,
    WaitForStateTransitionResult,
    GetConsensusParams,
    GetProtocolVersionUpgradeState,
    GetProtocolVersionUpgradeVoteStatus,
    GetEpochsInfo,
    GetContestedResources,
    GetContestedResourceVoteState,
    GetContestedResourceVotersForIdentity,
    GetContestedResourceIdentityVotes,
    GetVotePollsByEndDate,
    GetPrefundedSpecializedBalance,
    GetTotalCreditsInPlatform,
    GetPathElements,
    GetStatus,
    GetCurrentQuorumsInfo,
}

impl RequestTypeDto {
    pub fn from_backend(rt: RequestType) -> Self {
        match rt {
            RequestType::BroadcastStateTransition => Self::BroadcastStateTransition,
            RequestType::GetIdentity => Self::GetIdentity,
            RequestType::GetIdentityKeys => Self::GetIdentityKeys,
            RequestType::GetIdentitiesContractKeys => Self::GetIdentitiesContractKeys,
            RequestType::GetIdentityNonce => Self::GetIdentityNonce,
            RequestType::GetIdentityContractNonce => Self::GetIdentityContractNonce,
            RequestType::GetIdentityBalance => Self::GetIdentityBalance,
            RequestType::GetIdentitiesBalances => Self::GetIdentitiesBalances,
            RequestType::GetIdentityBalanceAndRevision => Self::GetIdentityBalanceAndRevision,
            RequestType::GetEvonodesProposedEpochBlocksByIds => {
                Self::GetEvonodesProposedEpochBlocksByIds
            }
            RequestType::GetEvonodesProposedEpochBlocksByRange => {
                Self::GetEvonodesProposedEpochBlocksByRange
            }
            RequestType::GetProofs => Self::GetProofs,
            RequestType::GetDataContract => Self::GetDataContract,
            RequestType::GetDataContractHistory => Self::GetDataContractHistory,
            RequestType::GetDataContracts => Self::GetDataContracts,
            RequestType::GetDocuments => Self::GetDocuments,
            RequestType::GetIdentityByPublicKeyHash => Self::GetIdentityByPublicKeyHash,
            RequestType::WaitForStateTransitionResult => Self::WaitForStateTransitionResult,
            RequestType::GetConsensusParams => Self::GetConsensusParams,
            RequestType::GetProtocolVersionUpgradeState => Self::GetProtocolVersionUpgradeState,
            RequestType::GetProtocolVersionUpgradeVoteStatus => {
                Self::GetProtocolVersionUpgradeVoteStatus
            }
            RequestType::GetEpochsInfo => Self::GetEpochsInfo,
            RequestType::GetContestedResources => Self::GetContestedResources,
            RequestType::GetContestedResourceVoteState => Self::GetContestedResourceVoteState,
            RequestType::GetContestedResourceVotersForIdentity => {
                Self::GetContestedResourceVotersForIdentity
            }
            RequestType::GetContestedResourceIdentityVotes => {
                Self::GetContestedResourceIdentityVotes
            }
            RequestType::GetVotePollsByEndDate => Self::GetVotePollsByEndDate,
            RequestType::GetPrefundedSpecializedBalance => Self::GetPrefundedSpecializedBalance,
            RequestType::GetTotalCreditsInPlatform => Self::GetTotalCreditsInPlatform,
            RequestType::GetPathElements => Self::GetPathElements,
            RequestType::GetStatus => Self::GetStatus,
            RequestType::GetCurrentQuorumsInfo => Self::GetCurrentQuorumsInfo,
        }
    }
}

/// A single proof log entry for the frontend.
///
/// Binary fields (request_bytes, proof_bytes, verification_path_query_bytes)
/// are hex-encoded so they serialize cleanly to JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProofLogItemDto {
    pub request_type: RequestTypeDto,
    pub request_bytes_hex: String,
    pub verification_path_query_hex: String,
    pub height: u64,
    pub time_ms: u64,
    pub proof_bytes_hex: String,
    pub error: Option<String>,
}

impl ProofLogItemDto {
    pub fn from_backend(item: ProofLogItem) -> Self {
        Self {
            request_type: RequestTypeDto::from_backend(item.request_type),
            request_bytes_hex: hex::encode(&item.request_bytes),
            verification_path_query_hex: hex::encode(&item.verification_path_query_bytes),
            height: item.height,
            time_ms: item.time_ms,
            proof_bytes_hex: hex::encode(&item.proof_bytes),
            error: item.error,
        }
    }
}

/// Input for paginated proof log queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProofLogQueryInput {
    /// If true, only return items that have a non-null error.
    pub only_errored: bool,
    /// Zero-based page number.
    pub page: u64,
    /// Number of items per page.
    pub items_per_page: u64,
}

/// Paginated response for proof log queries.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProofLogPageDto {
    pub items: Vec<ProofLogItemDto>,
    pub page: u64,
    pub items_per_page: u64,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Get a paginated page of proof log items from the database.
///
/// Items are sorted by `time_ms DESC` (newest first). Optionally filter
/// to only items that have a verification error.
#[tauri::command]
#[specta::specta]
pub fn proof_log_get_items(
    state: tauri::State<'_, Arc<AppState>>,
    input: ProofLogQueryInput,
) -> Result<ProofLogPageDto, String> {
    let db = state.db();
    let offset = input.page * input.items_per_page;
    let limit = input.items_per_page;
    let range = offset..(offset + limit);

    let items = db
        .get_proof_log_items(input.only_errored, range)
        .map_err(|e| format!("Failed to read proof log: {e}"))?;

    let dto_items: Vec<ProofLogItemDto> = items
        .into_iter()
        .map(ProofLogItemDto::from_backend)
        .collect();

    Ok(ProofLogPageDto {
        items: dto_items,
        page: input.page,
        items_per_page: input.items_per_page,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_type_dto_roundtrip_all_variants() {
        let variants = [
            (
                RequestType::BroadcastStateTransition,
                RequestTypeDto::BroadcastStateTransition,
            ),
            (RequestType::GetIdentity, RequestTypeDto::GetIdentity),
            (
                RequestType::GetIdentityKeys,
                RequestTypeDto::GetIdentityKeys,
            ),
            (
                RequestType::GetIdentitiesContractKeys,
                RequestTypeDto::GetIdentitiesContractKeys,
            ),
            (
                RequestType::GetIdentityNonce,
                RequestTypeDto::GetIdentityNonce,
            ),
            (
                RequestType::GetIdentityContractNonce,
                RequestTypeDto::GetIdentityContractNonce,
            ),
            (
                RequestType::GetIdentityBalance,
                RequestTypeDto::GetIdentityBalance,
            ),
            (
                RequestType::GetIdentitiesBalances,
                RequestTypeDto::GetIdentitiesBalances,
            ),
            (
                RequestType::GetIdentityBalanceAndRevision,
                RequestTypeDto::GetIdentityBalanceAndRevision,
            ),
            (
                RequestType::GetEvonodesProposedEpochBlocksByIds,
                RequestTypeDto::GetEvonodesProposedEpochBlocksByIds,
            ),
            (
                RequestType::GetEvonodesProposedEpochBlocksByRange,
                RequestTypeDto::GetEvonodesProposedEpochBlocksByRange,
            ),
            (RequestType::GetProofs, RequestTypeDto::GetProofs),
            (
                RequestType::GetDataContract,
                RequestTypeDto::GetDataContract,
            ),
            (
                RequestType::GetDataContractHistory,
                RequestTypeDto::GetDataContractHistory,
            ),
            (
                RequestType::GetDataContracts,
                RequestTypeDto::GetDataContracts,
            ),
            (RequestType::GetDocuments, RequestTypeDto::GetDocuments),
            (
                RequestType::GetIdentityByPublicKeyHash,
                RequestTypeDto::GetIdentityByPublicKeyHash,
            ),
            (
                RequestType::WaitForStateTransitionResult,
                RequestTypeDto::WaitForStateTransitionResult,
            ),
            (
                RequestType::GetConsensusParams,
                RequestTypeDto::GetConsensusParams,
            ),
            (
                RequestType::GetProtocolVersionUpgradeState,
                RequestTypeDto::GetProtocolVersionUpgradeState,
            ),
            (
                RequestType::GetProtocolVersionUpgradeVoteStatus,
                RequestTypeDto::GetProtocolVersionUpgradeVoteStatus,
            ),
            (RequestType::GetEpochsInfo, RequestTypeDto::GetEpochsInfo),
            (
                RequestType::GetContestedResources,
                RequestTypeDto::GetContestedResources,
            ),
            (
                RequestType::GetContestedResourceVoteState,
                RequestTypeDto::GetContestedResourceVoteState,
            ),
            (
                RequestType::GetContestedResourceVotersForIdentity,
                RequestTypeDto::GetContestedResourceVotersForIdentity,
            ),
            (
                RequestType::GetContestedResourceIdentityVotes,
                RequestTypeDto::GetContestedResourceIdentityVotes,
            ),
            (
                RequestType::GetVotePollsByEndDate,
                RequestTypeDto::GetVotePollsByEndDate,
            ),
            (
                RequestType::GetPrefundedSpecializedBalance,
                RequestTypeDto::GetPrefundedSpecializedBalance,
            ),
            (
                RequestType::GetTotalCreditsInPlatform,
                RequestTypeDto::GetTotalCreditsInPlatform,
            ),
            (
                RequestType::GetPathElements,
                RequestTypeDto::GetPathElements,
            ),
            (RequestType::GetStatus, RequestTypeDto::GetStatus),
            (
                RequestType::GetCurrentQuorumsInfo,
                RequestTypeDto::GetCurrentQuorumsInfo,
            ),
        ];

        for (backend, expected_dto) in &variants {
            let dto = RequestTypeDto::from_backend(*backend);
            assert_eq!(dto, *expected_dto, "Failed for {:?}", backend);
        }
    }

    #[test]
    fn proof_log_item_dto_from_backend() {
        let item = ProofLogItem {
            request_type: RequestType::GetIdentity,
            request_bytes: vec![0xde, 0xad, 0xbe, 0xef],
            verification_path_query_bytes: vec![0xca, 0xfe],
            height: 42,
            time_ms: 1700000000000,
            proof_bytes: vec![0x01, 0x02, 0x03],
            error: Some("proof mismatch".into()),
        };

        let dto = ProofLogItemDto::from_backend(item);

        assert_eq!(dto.request_type, RequestTypeDto::GetIdentity);
        assert_eq!(dto.request_bytes_hex, "deadbeef");
        assert_eq!(dto.verification_path_query_hex, "cafe");
        assert_eq!(dto.height, 42);
        assert_eq!(dto.time_ms, 1700000000000);
        assert_eq!(dto.proof_bytes_hex, "010203");
        assert_eq!(dto.error.as_deref(), Some("proof mismatch"));
    }

    #[test]
    fn proof_log_item_dto_from_backend_no_error() {
        let item = ProofLogItem {
            request_type: RequestType::GetDocuments,
            request_bytes: vec![],
            verification_path_query_bytes: vec![],
            height: 100,
            time_ms: 1700000001000,
            proof_bytes: vec![0xff],
            error: None,
        };

        let dto = ProofLogItemDto::from_backend(item);

        assert_eq!(dto.request_type, RequestTypeDto::GetDocuments);
        assert_eq!(dto.request_bytes_hex, "");
        assert_eq!(dto.verification_path_query_hex, "");
        assert_eq!(dto.height, 100);
        assert_eq!(dto.proof_bytes_hex, "ff");
        assert!(dto.error.is_none());
    }

    #[test]
    fn proof_log_query_input_serializes() {
        let input = ProofLogQueryInput {
            only_errored: true,
            page: 3,
            items_per_page: 50,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"onlyErrored\":true"));
        assert!(json.contains("\"page\":3"));
        assert!(json.contains("\"itemsPerPage\":50"));
    }

    #[test]
    fn proof_log_query_input_deserializes() {
        let json = r#"{"onlyErrored":false,"page":0,"itemsPerPage":100}"#;
        let input: ProofLogQueryInput = serde_json::from_str(json).unwrap();
        assert!(!input.only_errored);
        assert_eq!(input.page, 0);
        assert_eq!(input.items_per_page, 100);
    }

    #[test]
    fn proof_log_page_dto_serializes() {
        let page = ProofLogPageDto {
            items: vec![ProofLogItemDto {
                request_type: RequestTypeDto::GetIdentity,
                request_bytes_hex: "aa".into(),
                verification_path_query_hex: "bb".into(),
                height: 10,
                time_ms: 999,
                proof_bytes_hex: "cc".into(),
                error: None,
            }],
            page: 0,
            items_per_page: 100,
        };
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.contains("\"requestType\":\"getIdentity\""));
        assert!(json.contains("\"requestBytesHex\":\"aa\""));
        assert!(json.contains("\"height\":10"));
        assert!(json.contains("\"page\":0"));
    }

    #[test]
    fn proof_log_page_dto_empty_items() {
        let page = ProofLogPageDto {
            items: vec![],
            page: 5,
            items_per_page: 100,
        };
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.contains("\"items\":[]"));
        assert!(json.contains("\"page\":5"));
    }

    #[test]
    fn request_type_dto_serializes_camel_case() {
        let dto = RequestTypeDto::GetContestedResourceVotersForIdentity;
        let json = serde_json::to_string(&dto).unwrap();
        assert_eq!(json, "\"getContestedResourceVotersForIdentity\"");
    }
}
