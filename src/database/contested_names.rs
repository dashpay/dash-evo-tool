use crate::context::AppContext;
use crate::database::Database;
use crate::model::contested_name::{Contestant, ContestedName};
use dpp::identifier::Identifier;
use rusqlite::{params, Result};

impl Database {
    pub fn insert_or_update_name_contest(
        &self,
        contested_name: &ContestedName,
        app_context: &AppContext,
    ) -> Result<()> {
        let network = app_context.network_string();

        // Check if the contested name already exists and get the current values if it does
        let mut stmt = self.conn.prepare(
            "SELECT locked_votes, abstain_votes, awarded_to, ending_time
             FROM contested_name
             WHERE normalized_contested_name = ? AND network = ?",
        )?;
        let result = stmt.query_row(
            params![contested_name.normalized_contested_name, network],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<u64>>(3)?,
                ))
            },
        );

        match result {
            Ok((locked_votes, abstain_votes, awarded_to, ending_time)) => {
                // Compare the current values with the new values
                let should_update = locked_votes != contested_name.locked_votes
                    || abstain_votes != contested_name.abstain_votes
                    || awarded_to.as_ref().map(|id| {
                        Identifier::from_bytes(id).expect("expected 32 bytes for awarded to")
                    }) != contested_name.awarded_to
                    || ending_time != contested_name.ending_time;

                if should_update {
                    // Update the entry if any field has changed
                    self.conn.execute(
                        "UPDATE contested_name
                         SET locked_votes = ?, abstain_votes = ?, awarded_to = ?, ending_time = ?
                         WHERE normalized_contested_name = ? AND network = ?",
                        params![
                            contested_name.locked_votes,
                            contested_name.abstain_votes,
                            contested_name.awarded_to.as_ref().map(|id| id.to_vec()),
                            contested_name.ending_time,
                            contested_name.normalized_contested_name,
                            network,
                        ],
                    )?;
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // If the contested name doesn't exist, insert it
                self.conn.execute(
                    "INSERT INTO contested_name (normalized_contested_name, locked_votes, abstain_votes, awarded_to, ending_time, network)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        contested_name.normalized_contested_name,
                        contested_name.locked_votes,
                        contested_name.abstain_votes,
                        contested_name.awarded_to.as_ref().map(|id| id.to_vec()),
                        contested_name.ending_time,
                        network,
                    ],
                )?;
            }
            Err(e) => return Err(e.into()),
        }

        // Insert or update each contestant associated with the contested name
        for contestant in &contested_name.contestants {
            self.insert_or_update_contestant(
                &contested_name.normalized_contested_name,
                contestant,
                &network,
            )?;
        }

        Ok(())
    }

    pub fn insert_or_update_contestant(
        &self,
        contest_id: &str,
        contestant: &Contestant,
        network: &str,
    ) -> Result<()> {
        // Check if the contestant already exists and get the current values if it does
        let mut stmt = self.conn.prepare(
            "SELECT name, info, votes
             FROM contestant
             WHERE contest_id = ? AND identity_id = ? AND network = ?",
        )?;
        let result = stmt.query_row(
            params![contest_id, contestant.id.to_vec(), network],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        );

        match result {
            Ok((name, info, votes)) => {
                // Compare the current values with the new values
                let should_update =
                    name != contestant.name || info != contestant.info || votes != contestant.votes;

                if should_update {
                    // Update the entry if any field has changed
                    self.conn.execute(
                        "UPDATE contestant
                         SET name = ?, info = ?, votes = ?
                         WHERE contest_id = ? AND identity_id = ? AND network = ?",
                        params![
                            contestant.name,
                            contestant.info,
                            contestant.votes,
                            contest_id,
                            contestant.id.to_vec(),
                            network,
                        ],
                    )?;
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // If the contestant doesn't exist, insert it
                self.conn.execute(
                    "INSERT INTO contestant (contest_id, identity_id, name, info, votes, network)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        contest_id,
                        contestant.id.to_vec(),
                        contestant.name,
                        contestant.info,
                        contestant.votes,
                        network,
                    ],
                )?;
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }
}
