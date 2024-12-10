use crate::{
    backend_task::contested_names::schedule_dpns_vote::ScheduledDPNSVote, context::AppContext,
    database::Database,
};
use dash_sdk::{
    dpp::{
        platform_value::string_encoding::Encoding,
        voting::vote_choices::resource_vote_choice::ResourceVoteChoice,
    },
    platform::Identifier,
};
use rusqlite::params;

impl Database {
    pub fn initialize_scheduled_votes_table(&self) -> rusqlite::Result<()> {
        // Create the scheduled_votes table
        self.execute(
            "CREATE TABLE IF NOT EXISTS scheduled_votes (
                identity_id BLOB NOT NULL,
                contested_name STRING NOT NULL,
                vote_choice BLOB NOT NULL,
                time INTEGER NOT NULL,
                PRIMARY KEY (identity_id),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    pub fn insert_scheduled_vote(
        &self,
        identity_id: &[u8],
        contested_name: String,
        vote_choice: ResourceVoteChoice,
        time: u64,
    ) -> rusqlite::Result<()> {
        let vote_choice_string = vote_choice.to_string();
        self.execute(
            "INSERT INTO scheduled_votes (identity_id, contested_name, vote_choice, time) VALUES (?, ?, ?, ?)",
            params![identity_id, contested_name, vote_choice_string, time],
        )?;
        Ok(())
    }

    pub fn get_scheduled_votes(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<ScheduledDPNSVote>> {
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM scheduled_votes WHERE network = ?")?;
        let votes_iter = stmt.query_map(params![network], |row| {
            let voter_id_bytes: Vec<u8> = row.get(0)?;
            let contested_name: String = row.get(1)?;
            let vote_choice_string: String = row.get(2)?;
            let time: u64 = row.get(3)?;
            let vote_choice = match vote_choice_string.as_str() {
                "Abstain" => ResourceVoteChoice::Abstain,
                "Lock" => ResourceVoteChoice::Lock,
                other => {
                    if let Some(inner) = other.strip_prefix("TowardsIdentity(") {
                        if let Some(inner) = inner.strip_suffix(')') {
                            let towards_id = inner.to_string();
                            ResourceVoteChoice::TowardsIdentity(
                                Identifier::from_string(&towards_id, Encoding::Base58)
                                    .expect("Expected valid identifier"),
                            )
                        } else {
                            return Err(rusqlite::Error::InvalidQuery);
                        }
                    } else {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                }
            };
            let scheduled_vote = ScheduledDPNSVote {
                voter_id: Identifier::from_bytes(&voter_id_bytes)
                    .expect("Expected valid identifier"),
                contested_name,
                choice: vote_choice,
                time,
            };

            Ok(scheduled_vote)
        })?;

        let scheduled_votes: rusqlite::Result<Vec<ScheduledDPNSVote>> = votes_iter.collect();
        scheduled_votes
    }
}
