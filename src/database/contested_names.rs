use crate::context::AppContext;
use crate::database::Database;
use crate::model::contested_name::{Contestant, ContestedName};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight};
use dash_sdk::query_types::Contenders;
use rusqlite::{params, params_from_iter, Result};
use std::collections::{BTreeMap, HashMap, HashSet};

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
                cn.last_updated,
                c.identity_id,
                c.name,
                c.votes,
                c.created_at,
                c.created_at_block_height,
                c.created_at_core_block_height,
                c.document_id,
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
            let locked_votes: Option<u32> = row.get(1)?;
            let abstain_votes: Option<u32> = row.get(2)?;
            let awarded_to: Option<Vec<u8>> = row.get(3)?;
            let ending_time: Option<u64> = row.get(4)?;
            let last_updated: Option<u64> = row.get(5)?;
            let identity_id: Option<Vec<u8>> = row.get(6)?;
            let contestant_name: Option<String> = row.get(7)?;
            let votes: Option<u32> = row.get(8)?;
            let created_at: Option<TimestampMillis> = row.get(9)?;
            let created_at_block_height: Option<BlockHeight> = row.get(10)?;
            let created_at_core_block_height: Option<CoreBlockHeight> = row.get(11)?;
            let document_id: Option<Vec<u8>> = row.get(12)?;
            let identity_info: Option<String> = row.get(13)?;

            // Convert `awarded_to` to `Identifier` if it exists
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
                    contestants: Some(Vec::new()), // Initialize as an empty vector
                    last_updated,
                    my_votes: BTreeMap::new(), // Assuming this is filled elsewhere
                });

            // If there are contestant details in the row, add them
            if let (Some(identity_id), Some(contestant_name), Some(votes), Some(document_id)) =
                (identity_id, contestant_name, votes, document_id)
            {
                let contestant = Contestant {
                    id: Identifier::from_bytes(&identity_id)
                        .expect("Expected 32 bytes for identity_id"),
                    name: contestant_name,
                    info: identity_info.unwrap_or_default(),
                    votes,
                    created_at,
                    created_at_block_height,
                    created_at_core_block_height,
                    document_id: Identifier::from_bytes(&document_id)
                        .expect("Expected 32 bytes for document_id"),
                };

                // Add the contestant to the contestants list
                if let Some(contestants) = &mut contested_name.contestants {
                    contestants.push(contestant);
                }
            }

            Ok(())
        })?;

        // Ensure all rows are processed without error
        for row in rows {
            row?;
        }

        // Collect the values from the hashmap and return as a vector
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
                    row.get::<_, Option<u32>>(0)?,
                    row.get::<_, Option<u32>>(1)?,
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
            Err(e) => return Err(e),
        }

        // If there are contestants, insert or update each contestant associated with the contested name
        if let Some(contestants) = &contested_name.contestants {
            for contestant in contestants {
                self.insert_or_update_contestant(
                    &contested_name.normalized_contested_name,
                    contestant,
                    app_context,
                )?;
            }
        }

        Ok(())
    }

    pub fn insert_or_update_contenders(
        &self,
        contest_id: &str,
        contenders: &Contenders,
        dpns_domain_document_type: DocumentTypeRef,
        app_context: &AppContext,
    ) -> Result<()> {
        if contenders.winner.is_some() {
            return Ok(()); //todo
        }
        let network = app_context.network_string();
        let mut conn = self.conn.lock().unwrap();
        let locked_votes = contenders.lock_vote_tally.unwrap_or(0) as i64;
        let abstain_votes = contenders.abstain_vote_tally.unwrap_or(0) as i64;
        let last_updated = chrono::Utc::now().timestamp(); // Get the current timestamp

        // Start a transaction
        let tx = conn.transaction()?;

        // Update the `contested_name` table with locked votes, abstain votes, and last updated
        tx.execute(
            "UPDATE contested_name
         SET locked_votes = ?, abstain_votes = ?, last_updated = ?
         WHERE normalized_contested_name = ? AND network = ?",
            params![
                locked_votes,
                abstain_votes,
                last_updated,
                contest_id,
                network
            ],
        )?;

        // Iterate over each contender in the Contenders struct
        for (identity_id, contender) in &contenders.contenders {
            // Convert the identity ID to bytes
            let identity_id_bytes = identity_id.to_vec();

            // Serialize the document if available
            let deserialized_contender = contender
                .try_to_contender(dpns_domain_document_type, app_context.platform_version)
                .expect("expect a contender document deserialization");

            let document = deserialized_contender.document().as_ref().unwrap().clone();

            let name = document
                .get("label")
                .expect("expected name")
                .as_str()
                .unwrap();

            let created_at = document.created_at();
            let created_at_block_height = document.created_at_block_height();
            let created_at_core_block_height = document.created_at_core_block_height();
            let document_id = document.id();

            // Check if the contender already exists
            let mut stmt = tx.prepare(
                "SELECT votes
             FROM contestant
             WHERE contest_id = ? AND identity_id = ? AND network = ?",
            )?;

            let result = stmt.query_row(
                params![contest_id, identity_id_bytes.clone(), network],
                |row| row.get::<_, u64>(0),
            );

            match result {
                Ok(current_votes) => {
                    // Update the existing entry if votes or serialized document are different
                    if current_votes != contender.vote_tally().unwrap_or(0) as u64 {
                        tx.execute(
                            "UPDATE contestant
                         SET votes = ?
                         WHERE contest_id = ? AND identity_id = ? AND network = ?",
                            params![
                                contender.vote_tally().unwrap_or(0),
                                contest_id,
                                identity_id_bytes,
                                network,
                            ],
                        )?;
                    }
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    // If the contestant doesn't exist, insert it
                    tx.execute(
                        "INSERT INTO contestant (contest_id, identity_id, name, votes, created_at, created_at_block_height, created_at_core_block_height, document_id, network)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                        contest_id,
                        identity_id_bytes,
                        name,
                        contender.vote_tally().unwrap_or(0),
                        created_at,
                        created_at_block_height,
                        created_at_core_block_height,
                        document_id.to_vec(),
                        network,
                    ],
                    )?;
                }
                Err(e) => return Err(e),
            }
        }

        // Commit the transaction
        tx.commit()?;

        Ok(())
    }

    pub fn insert_or_update_contestant(
        &self,
        contest_id: &str,
        contestant: &Contestant,
        app_context: &AppContext,
    ) -> Result<()> {
        let network = app_context.network_string();
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
                    row.get::<_, u32>(2)?,
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
            Err(e) => return Err(e),
        }

        Ok(())
    }

    pub fn insert_name_contests_as_normalized_names(
        &self,
        name_contests: Vec<String>,
        app_context: &AppContext,
    ) -> Result<Vec<String>> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();
        let mut names_to_be_updated: Vec<(String, Option<i64>)> = Vec::new();
        let mut new_names: Vec<String> = Vec::new();

        // Define the time limit (one hour ago in Unix timestamp format)
        let one_hour_ago = chrono::Utc::now().timestamp() - 3600;
        let two_weeks_ago = chrono::Utc::now().timestamp() - 1_209_600;

        // Chunk the name_contests into smaller groups due to SQL parameter limits
        let chunk_size = 900; // Use a safe limit to stay below SQLite's limit

        for chunk in name_contests.chunks(chunk_size) {
            // Prepare placeholders for the SQL IN clause
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            let query = format!(
                "SELECT normalized_contested_name, last_updated
             FROM contested_name
             WHERE network = ? AND normalized_contested_name IN ({})",
                placeholders
            );

            let mut stmt = conn.prepare(&query)?;

            // Create params: network followed by each name in the chunk
            let mut params: Vec<&dyn rusqlite::ToSql> = vec![&network];
            for name in chunk {
                params.push(name);
            }

            // Execute the query and collect outdated or never updated names
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                let name: String = row.get(0)?;
                let last_updated: Option<i64> = row.get(1)?;
                Ok((name, last_updated))
            })?;

            // Track the existing and outdated names
            let mut existing_names = HashSet::new();
            for row in rows {
                if let Ok((name, last_updated)) = row {
                    existing_names.insert(name.clone());
                    if last_updated.is_none()
                        || (app_context.network == Network::Testnet
                            && last_updated.unwrap() < one_hour_ago)
                        || (app_context.network == Network::Dash
                            && last_updated.unwrap() < two_weeks_ago)
                    {
                        names_to_be_updated.push((name, last_updated));
                    }
                }
            }

            // Identify and collect new names (those not in existing_names)
            for name in chunk {
                if !existing_names.contains(name) {
                    new_names.push(name.clone());
                }
            }
        }

        // Insert new names into the database
        if !new_names.is_empty() {
            let mut insert_stmt = conn.prepare(
                "INSERT INTO contested_name (normalized_contested_name, network, winner_type)
             VALUES (?, ?, 0)",
            )?;

            for name in &new_names {
                insert_stmt.execute(params![name, network])?;
            }
        }

        // Combine the new names and outdated names, sorted by last_updated (oldest first)
        names_to_be_updated.extend(new_names.into_iter().map(|name| (name, None)));
        names_to_be_updated.sort_by(|a, b| a.1.unwrap_or(0).cmp(&b.1.unwrap_or(0)));

        // Extract the names into a Vec<String>
        let result_names = names_to_be_updated
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();

        Ok(result_names)
    }

    pub fn update_ending_time<I>(&self, name_contests: I, app_context: &AppContext) -> Result<()>
    where
        I: IntoIterator<Item = (String, TimestampMillis)>,
    {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();

        // Prepare statement for selecting existing entries
        let select_query = "SELECT ending_time
                    FROM contested_name
                    WHERE network = ? AND normalized_contested_name = ?";

        let mut select_stmt = conn.prepare(select_query)?;

        // Prepare statement for updating existing entries
        let update_query = "UPDATE contested_name
                    SET ending_time = ?
                    WHERE normalized_contested_name = ? AND network = ?";
        let mut update_stmt = conn.prepare(update_query)?;

        for (name, new_ending_time) in name_contests {
            // Check if the name exists in the database and retrieve the current ending time
            let existing_ending_time: Option<TimestampMillis> =
                select_stmt.query_row(params![network, name], |row| {
                    let ending_time: Result<Option<TimestampMillis>> = row.get(0);
                    ending_time
                })?;

            if let Some(existing_ending_time) = existing_ending_time {
                // Update only if the new ending time is greater than the existing one
                if existing_ending_time < new_ending_time {
                    update_stmt.execute(params![new_ending_time, name, network])?;
                }
            } else {
                // If `ending_time` is `NULL`, update with the new ending time
                update_stmt.execute(params![new_ending_time, name, network])?;
            }
        }

        Ok(())
    }
}
