use crate::context::AppContext;
use crate::database::Database;
use crate::model::contested_name::{Contestant, ContestedName};
use dash_sdk::dpp::identifier::Identifier;
use rusqlite::{params, Result};
use std::collections::{BTreeMap, HashMap};

impl Database {
    pub fn get_contested_names(&self, app_context: &AppContext) -> Result<Vec<ContestedName>> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                cn.normalized_contested_name,
                cn.locked_votes,
                cn.abstain_votes,
                cn.awarded_to,
                cn.ending_time,
                c.identity_id,
                c.name,
                c.votes,
                i.info
             FROM contested_name cn
             LEFT JOIN contestant c
             ON cn.normalized_contested_name = c.contest_id
             AND cn.network = c.network
             LEFT JOIN identity i
             ON c.identity_id = i.id
             AND c.network = i.network
             WHERE cn.network = ?",
        )?;

        // A hashmap to collect contested names, keyed by their normalized name
        let mut contested_name_map: HashMap<String, ContestedName> = HashMap::new();

        // Iterate over the joined rows
        let rows = stmt.query_map(params![network], |row| {
            let normalized_contested_name: String = row.get(0)?;
            let locked_votes: u64 = row.get(1)?;
            let abstain_votes: u64 = row.get(2)?;
            let awarded_to: Option<Vec<u8>> = row.get(3)?;
            let ending_time: Option<u64> = row.get(4)?;
            let identity_id: Option<Vec<u8>> = row.get(5)?;
            let contestant_name: Option<String> = row.get(6)?;
            let votes: Option<u64> = row.get(7)?;
            let identity_info: Option<String> = row.get(8)?;

            let awarded_to_id = awarded_to
                .map(|id| Identifier::from_bytes(&id).expect("Expected 32 bytes for awarded_to"));

            // Create or get the contested name from the hashmap
            let contested_name = contested_name_map
                .entry(normalized_contested_name.clone())
                .or_insert(ContestedName {
                    normalized_contested_name: normalized_contested_name.clone(),
                    locked_votes,
                    abstain_votes,
                    awarded_to: awarded_to_id,
                    ending_time,
                    contestants: Vec::new(),
                    my_votes: BTreeMap::new(), // Assuming this is filled elsewhere
                });

            // If there are contestant details in the row, add them
            if let (Some(identity_id), Some(contestant_name), Some(votes)) =
                (identity_id, contestant_name, votes)
            {
                let contestant = Contestant {
                    id: Identifier::from_bytes(&identity_id)
                        .expect("Expected 32 bytes for identity_id"),
                    name: contestant_name,
                    info: identity_info.unwrap_or_default(),
                    votes,
                };
                contested_name.contestants.push(contestant);
            }

            Ok(())
        })?;

        // Iterate over rows to populate contested names and contestants
        for row_result in rows {
            row_result?;
        }

        // Collect the values from the hashmap into a vector and return it
        Ok(contested_name_map.into_values().collect())
    }

    pub fn insert_or_update_name_contest(
        &self,
        contested_name: &ContestedName,
        app_context: &AppContext,
    ) -> Result<()> {
        let network = app_context.network_string();

        // Check if the contested name already exists and get the current values if it does
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
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
                    self.execute(
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
                self.execute(
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
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
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
                    self.execute(
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
                self.execute(
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
