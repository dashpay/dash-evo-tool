use crate::model::qualified_identity::EncryptedPrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{KeyID, TimestampMillis};
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight, Identifier};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::collections::BTreeMap;

#[derive(Debug, Encode, Decode, Clone)]
pub enum ContestState {
    Unknown,
    Joinable,
    Ongoing,
    WonBy(Identifier),
    Locked,
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct ContestedName {
    pub normalized_contested_name: String,
    pub contestants: Option<Vec<Contestant>>,
    pub locked_votes: Option<u32>,
    pub abstain_votes: Option<u32>,
    pub awarded_to: Option<Identifier>,
    pub end_time: Option<TimestampMillis>,
    pub state: ContestState,
    pub last_updated: Option<TimestampMillis>,
    pub my_votes: BTreeMap<(Identifier, EncryptedPrivateKeyTarget, KeyID), ResourceVoteChoice>,
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct Contestant {
    pub id: Identifier,
    pub name: String,
    pub info: String,
    pub votes: u32,
    pub created_at: Option<TimestampMillis>,
    pub created_at_block_height: Option<BlockHeight>,
    pub created_at_core_block_height: Option<CoreBlockHeight>,
    pub document_id: Identifier,
}
