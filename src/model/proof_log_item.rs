#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub enum RequestType {
    BroadcastStateTransition = 1,
    GetIdentity = 2,
    GetIdentityKeys = 3,
    GetIdentitiesContractKeys = 4,
    GetIdentityNonce = 5,
    GetIdentityContractNonce = 6,
    GetIdentityBalance = 7,
    GetIdentitiesBalances = 8,
    GetIdentityBalanceAndRevision = 9,
    GetEvonodesProposedEpochBlocksByIds = 10,
    GetEvonodesProposedEpochBlocksByRange = 11,
    GetProofs = 12,
    GetDataContract = 13,
    GetDataContractHistory = 14,
    GetDataContracts = 15,
    GetDocuments = 16,
    GetIdentityByPublicKeyHash = 17,
    WaitForStateTransitionResult = 18,
    GetConsensusParams = 19,
    GetProtocolVersionUpgradeState = 20,
    GetProtocolVersionUpgradeVoteStatus = 21,
    GetEpochsInfo = 22,
    GetContestedResources = 23,
    GetContestedResourceVoteState = 24,
    GetContestedResourceVotersForIdentity = 25,
    GetContestedResourceIdentityVotes = 26,
    GetVotePollsByEndDate = 27,
    GetPrefundedSpecializedBalance = 28,
    GetTotalCreditsInPlatform = 29,
    GetPathElements = 30,
    GetStatus = 31,
    GetCurrentQuorumsInfo = 32,
}

use std::convert::TryFrom;

impl From<RequestType> for u8 {
    fn from(request_type: RequestType) -> Self {
        request_type as u8
    }
}

impl TryFrom<u8> for RequestType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(RequestType::BroadcastStateTransition),
            2 => Ok(RequestType::GetIdentity),
            3 => Ok(RequestType::GetIdentityKeys),
            4 => Ok(RequestType::GetIdentitiesContractKeys),
            5 => Ok(RequestType::GetIdentityNonce),
            6 => Ok(RequestType::GetIdentityContractNonce),
            7 => Ok(RequestType::GetIdentityBalance),
            8 => Ok(RequestType::GetIdentitiesBalances),
            9 => Ok(RequestType::GetIdentityBalanceAndRevision),
            10 => Ok(RequestType::GetEvonodesProposedEpochBlocksByIds),
            11 => Ok(RequestType::GetEvonodesProposedEpochBlocksByRange),
            12 => Ok(RequestType::GetProofs),
            13 => Ok(RequestType::GetDataContract),
            14 => Ok(RequestType::GetDataContractHistory),
            15 => Ok(RequestType::GetDataContracts),
            16 => Ok(RequestType::GetDocuments),
            17 => Ok(RequestType::GetIdentityByPublicKeyHash),
            18 => Ok(RequestType::WaitForStateTransitionResult),
            19 => Ok(RequestType::GetConsensusParams),
            20 => Ok(RequestType::GetProtocolVersionUpgradeState),
            21 => Ok(RequestType::GetProtocolVersionUpgradeVoteStatus),
            22 => Ok(RequestType::GetEpochsInfo),
            23 => Ok(RequestType::GetContestedResources),
            24 => Ok(RequestType::GetContestedResourceVoteState),
            25 => Ok(RequestType::GetContestedResourceVotersForIdentity),
            26 => Ok(RequestType::GetContestedResourceIdentityVotes),
            27 => Ok(RequestType::GetVotePollsByEndDate),
            28 => Ok(RequestType::GetPrefundedSpecializedBalance),
            29 => Ok(RequestType::GetTotalCreditsInPlatform),
            30 => Ok(RequestType::GetPathElements),
            31 => Ok(RequestType::GetStatus),
            32 => Ok(RequestType::GetCurrentQuorumsInfo),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProofLogItem {
    pub request_type: RequestType,
    pub request_bytes: Vec<u8>,
    pub verification_path_query_bytes: Vec<u8>,
    pub height: u64,
    pub time_ms: u64,
    pub proof_bytes: Vec<u8>,
    pub error: Option<String>,
}
