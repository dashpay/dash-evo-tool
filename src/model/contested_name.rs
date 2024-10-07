use crate::model::qualified_identity::EncryptedPrivateKeyTarget;
use bincode::{Decode, Encode};
use dpp::identity::{KeyID, TimestampMillis};
use dpp::prelude::Identifier;
use dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::collections::BTreeMap;

#[derive(Debug, Encode, Decode, Clone)]
pub struct ContestedName {
    pub normalized_contested_name: String,
    pub contestants: Vec<Contestant>,
    pub locked_votes: u64,
    pub abstain_votes: u64,
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
