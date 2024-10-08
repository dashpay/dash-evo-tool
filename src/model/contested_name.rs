use crate::model::qualified_identity::EncryptedPrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{KeyID, TimestampMillis};
use dash_sdk::dpp::prelude::Identifier;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::collections::BTreeMap;

#[derive(Debug, Encode, Decode, Clone)]
pub struct ContestedName {
    pub normalized_contested_name: String,
    pub contestants: Option<Vec<Contestant>>,
    pub locked_votes: Option<u64>,
    pub abstain_votes: Option<u64>,
    pub awarded_to: Option<Identifier>,
    pub ending_time: Option<TimestampMillis>,
    pub my_votes: BTreeMap<(Identifier, EncryptedPrivateKeyTarget, KeyID), ResourceVoteChoice>,
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct Contestant {
    pub id: Identifier,
    pub name: String,
    pub info: String,
    pub votes: u64,
}
