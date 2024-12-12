use crate::{
    backend_task::contested_names::ScheduledDPNSVote, context::AppContext, database::Database,
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
                contested_name TEXT NOT NULL,
                vote_choice TEXT NOT NULL,
                time INTEGER NOT NULL,
                executed INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                PRIMARY KEY (identity_id, contested_name)
            )",
            [],
        )?;
        Ok(())
    }

    pub fn insert_scheduled_votes(
        &self,
        app_context: &AppContext,
        votes: &Vec<ScheduledDPNSVote>,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        for vote in votes {
            let vote_choice = vote.choice.to_string();
            self.execute(
                "INSERT OR REPLACE INTO scheduled_votes (identity_id, contested_name, vote_choice, time, executed, network) VALUES (?, ?, ?, ?, 0, ?)",
                params![vote.voter_id.as_slice(), vote.contested_name, vote_choice, vote.unix_timestamp, network],
            )?;
        }
        Ok(())
    }

    pub fn delete_scheduled_vote(
        &self,
        app_context: &AppContext,
        identity_id: &[u8],
        contested_name: &str,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM scheduled_votes WHERE identity_id = ? AND contested_name = ? AND network = ?",
            params![identity_id, contested_name, network],
        )?;
        Ok(())
    }

    pub fn mark_vote_executed(
        &self,
        app_context: &AppContext,
        identity_id: &[u8],
        contested_name: String,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        self.execute(
            "UPDATE scheduled_votes SET executed = 1 WHERE identity_id = ? AND contested_name = ? AND network = ?",
            params![identity_id, contested_name, network],
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
            let executed_successfully: bool = match row.get(4)? {
                0 => false,
                1 => true,
                _ => unreachable!(),
            };

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
                unix_timestamp: time,
                executed_successfully,
            };

            Ok(scheduled_vote)
        })?;

        let scheduled_votes: rusqlite::Result<Vec<ScheduledDPNSVote>> = votes_iter.collect();
        scheduled_votes
    }

    /// Clear all scheduled votes from the db
    pub fn clear_all_scheduled_votes(&self, app_context: &AppContext) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM scheduled_votes WHERE network = ?",
            params![network],
        )?;

        Ok(())
    }

    pub fn clear_executed_scheduled_votes(&self, app_context: &AppContext) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "DELETE FROM scheduled_votes WHERE executed = 1 AND network = ?",
            params![network],
        )?;

        Ok(())
    }
}
