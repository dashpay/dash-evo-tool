use crate::context::AppContext;
use crate::database::Database;
use crate::model::contested_name::{ContestState, Contestant, ContestedName};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::document_type::DocumentTypeRef;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
use dash_sdk::query_types::Contenders;
use rusqlite::{params, params_from_iter, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;
use tracing::{error, info};

impl Database {
    pub fn get_all_contested_names(&self, app_context: &AppContext) -> Result<Vec<ContestedName>> {
        let network = app_context.network_string();
        let contest_duration = if app_context.network == Network::Dash {
            Duration::from_secs(60 * 60 * 24 * 14)
        } else {
            Duration::from_secs(60 * 90)
        };
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                cn.normalized_contested_name,
                cn.locked_votes,
                cn.abstain_votes,
                cn.awarded_to,
                cn.end_time,
                cn.locked,
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
             ON cn.normalized_contested_name = c.normalized_contested_name
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
            let locked: bool = row.get(5)?;
            let last_updated: Option<u64> = row.get(6)?;
            let identity_id: Option<Vec<u8>> = row.get(7)?;
            let contestant_name: Option<String> = row.get(8)?;
            let votes: Option<u32> = row.get(9)?;
            let created_at: Option<TimestampMillis> = row.get(10)?;
            let created_at_block_height: Option<BlockHeight> = row.get(11)?;
            let created_at_core_block_height: Option<CoreBlockHeight> = row.get(12)?;
            let document_id: Option<Vec<u8>> = row.get(13)?;
            let identity_info: Option<String> = row.get(14)?;

            // Convert `awarded_to` to `Identifier` if it exists
            let awarded_to_id = awarded_to
                .map(|id| Identifier::from_bytes(&id).expect("Expected 32 bytes for awarded_to"));

            let state = if locked {
                ContestState::Locked
            } else if let Some(awarded_to_id) = awarded_to_id {
                ContestState::WonBy(awarded_to_id)
            } else if let Some(created_at) = created_at {
                let elapsed_time = Duration::from_millis(
                    (std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64)
                        .saturating_sub(created_at),
                );

                if elapsed_time <= contest_duration / 2 {
                    ContestState::Joinable
                } else {
                    ContestState::Ongoing
                }
            } else {
                ContestState::Unknown
            };

            // Create or get the contested name from the hashmap
            let contested_name = contested_name_map
                .entry(normalized_contested_name.clone())
                .or_insert(ContestedName {
                    normalized_contested_name: normalized_contested_name.clone(),
                    locked_votes,
                    abstain_votes,
                    awarded_to: awarded_to_id,
                    end_time: ending_time,
                    contestants: Some(Vec::new()), // Initialize as an empty vector
                    last_updated,
                    my_votes: BTreeMap::new(), // Assuming this is filled elsewhere
                    state,
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

    pub fn get_ongoing_contested_names(
        &self,
        app_context: &AppContext,
    ) -> Result<Vec<ContestedName>> {
        let network = app_context.network_string();
        let contest_duration = if app_context.network == Network::Dash {
            Duration::from_secs(60 * 60 * 24 * 14)
        } else {
            Duration::from_secs(60 * 90)
        };
        let current_timestamp = std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                cn.normalized_contested_name,
                cn.locked_votes,
                cn.abstain_votes,
                cn.awarded_to,
                cn.end_time,
                cn.locked,
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
             ON cn.normalized_contested_name = c.normalized_contested_name
             AND cn.network = c.network
             LEFT JOIN identity i
             ON c.identity_id = i.id
             AND c.network = i.network
             WHERE cn.network = ?
             AND (cn.end_time IS NULL OR cn.end_time > ?)",
        )?;

        // A hashmap to collect contested names, keyed by their normalized name
        let mut contested_name_map: HashMap<String, ContestedName> = HashMap::new();

        // Iterate over the joined rows
        let rows = stmt.query_map(params![network, current_timestamp], |row| {
            let normalized_contested_name: String = row.get(0)?;
            let locked_votes: Option<u32> = row.get(1)?;
            let abstain_votes: Option<u32> = row.get(2)?;
            let awarded_to: Option<Vec<u8>> = row.get(3)?;
            let ending_time: Option<u64> = row.get(4)?;
            let locked: bool = row.get(5)?;
            let last_updated: Option<u64> = row.get(6)?;
            let identity_id: Option<Vec<u8>> = row.get(7)?;
            let contestant_name: Option<String> = row.get(8)?;
            let votes: Option<u32> = row.get(9)?;
            let created_at: Option<TimestampMillis> = row.get(10)?;
            let created_at_block_height: Option<BlockHeight> = row.get(11)?;
            let created_at_core_block_height: Option<CoreBlockHeight> = row.get(12)?;
            let document_id: Option<Vec<u8>> = row.get(13)?;
            let identity_info: Option<String> = row.get(14)?;

            // Convert `awarded_to` to `Identifier` if it exists
            let awarded_to_id = awarded_to
                .map(|id| Identifier::from_bytes(&id).expect("Expected 32 bytes for awarded_to"));

            let state = if locked {
                ContestState::Locked
            } else if let Some(awarded_to_id) = awarded_to_id {
                ContestState::WonBy(awarded_to_id)
            } else if let Some(created_at) = created_at {
                let elapsed_time = Duration::from_millis(
                    (std::time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64)
                        .saturating_sub(created_at),
                );

                if elapsed_time <= contest_duration / 2 {
                    ContestState::Joinable
                } else {
                    ContestState::Ongoing
                }
            } else {
                ContestState::Unknown
            };

            // Create or get the contested name from the hashmap
            let contested_name = contested_name_map
                .entry(normalized_contested_name.clone())
                .or_insert(ContestedName {
                    normalized_contested_name: normalized_contested_name.clone(),
                    locked_votes,
                    abstain_votes,
                    awarded_to: awarded_to_id,
                    end_time: ending_time,
                    contestants: Some(Vec::new()), // Initialize as an empty vector
                    last_updated,
                    my_votes: BTreeMap::new(), // Assuming this is filled elsewhere
                    state,
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
                    || ending_time != contested_name.end_time;

                if should_update {
                    // Update the entry if any field has changed
                    self.execute(
                        "UPDATE contested_name
                     SET locked_votes = ?, abstain_votes = ?, awarded_to = ?, end_time = ?
                     WHERE normalized_contested_name = ? AND network = ?",
                        params![
                            contested_name.locked_votes,
                            contested_name.abstain_votes,
                            contested_name.awarded_to.as_ref().map(|id| id.to_vec()),
                            contested_name.end_time,
                            contested_name.normalized_contested_name,
                            network,
                        ],
                    )?;
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // If the contested name doesn't exist, insert it
                self.execute(
                    "INSERT INTO contested_name (normalized_contested_name, locked_votes, abstain_votes, awarded_to, end_time, network)
                 VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                    contested_name.normalized_contested_name,
                    contested_name.locked_votes,
                    contested_name.abstain_votes,
                    contested_name.awarded_to.as_ref().map(|id| id.to_vec()),
                    contested_name.end_time,
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
        normalized_contested_name: &str,
        contenders: &Contenders,
        dpns_domain_document_type: DocumentTypeRef,
        app_context: &AppContext,
    ) -> Result<()> {
        let network = app_context.network_string();
        let last_updated = chrono::Utc::now().timestamp(); // Get the current timestamp
        if let Some((winner, block_info)) = contenders.winner {
            match winner {
                ContestedDocumentVotePollWinnerInfo::NoWinner => {}
                ContestedDocumentVotePollWinnerInfo::WonByIdentity(won_by) => {
                    let mut conn = self.conn.lock().unwrap();
                    // Start a transaction
                    let tx = conn.transaction()?;
                    tx.execute(
                        "UPDATE contested_name
         SET awarded_to = ?, last_updated = ?, end_time = ?
         WHERE normalized_contested_name = ? AND network = ?",
                        params![
                            won_by.to_vec(),
                            last_updated,
                            block_info.time_ms,
                            normalized_contested_name,
                            network,
                        ],
                    )?;
                    tx.commit()?;
                }
                ContestedDocumentVotePollWinnerInfo::Locked => {
                    let mut conn = self.conn.lock().unwrap();
                    // Start a transaction
                    let tx = conn.transaction()?;
                    tx.execute(
                        "UPDATE contested_name
         SET locked = 1, last_updated = ?, end_time = ?
         WHERE normalized_contested_name = ? AND network = ?",
                        params![
                            last_updated,
                            block_info.time_ms,
                            normalized_contested_name,
                            network,
                        ],
                    )?;
                    tx.commit()?;
                }
            }
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        let locked_votes = contenders.lock_vote_tally.unwrap_or(0) as i64;
        let abstain_votes = contenders.abstain_vote_tally.unwrap_or(0) as i64;

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
                normalized_contested_name,
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
             WHERE normalized_contested_name = ? AND identity_id = ? AND network = ?",
            )?;

            let result = stmt.query_row(
                params![
                    normalized_contested_name,
                    identity_id_bytes.clone(),
                    network
                ],
                |row| row.get::<_, u64>(0),
            );

            match result {
                Ok(current_votes) => {
                    // Update the existing entry if votes or serialized document are different
                    if current_votes != contender.vote_tally().unwrap_or(0) as u64 {
                        tx.execute(
                            "UPDATE contestant
                         SET votes = ?
                         WHERE normalized_contested_name = ? AND identity_id = ? AND network = ?",
                            params![
                                contender.vote_tally().unwrap_or(0),
                                normalized_contested_name,
                                identity_id_bytes,
                                network,
                            ],
                        )?;
                    }
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    // If the contestant doesn't exist, insert it
                    tx.execute(
                        "INSERT INTO contestant (normalized_contested_name, identity_id, name, votes, created_at, created_at_block_height, created_at_core_block_height, document_id, network)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                        normalized_contested_name,
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
        if let Err(e) = tx.commit() {
            error!("Transaction failed to commit: {:?}", e);
            return Err(e);
        }

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
        let half_a_minute_ago = chrono::Utc::now().timestamp() - 30;

        // Chunk the name_contests into smaller groups due to SQL parameter limits
        let chunk_size = 900; // Use a safe limit to stay below SQLite's limit

        for chunk in name_contests.chunks(chunk_size) {
            // Prepare placeholders for the SQL IN clause
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            let query = format!(
                "SELECT normalized_contested_name, last_updated
             FROM contested_name
             WHERE network = ? AND normalized_contested_name IN ({}) and awarded_to IS NULL",
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
                    if last_updated.is_none() || last_updated.unwrap() < half_a_minute_ago {
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
                "INSERT INTO contested_name (normalized_contested_name, network)
             VALUES (?, ?)",
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
        let select_query = "SELECT end_time
                    FROM contested_name
                    WHERE network = ? AND normalized_contested_name = ?";

        let mut select_stmt = conn.prepare(select_query)?;

        // Prepare statement for updating existing entries
        let update_query = "UPDATE contested_name
                    SET end_time = ?
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
    pub fn update_vote_count(
        &self,
        contested_name: &str,
        network: &str,
        vote_strength: u64,
        vote_choice: ResourceVoteChoice,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        match vote_choice {
            ResourceVoteChoice::TowardsIdentity(identity) => {
                // Increment the contestant's vote count
                tx.execute(
                    "UPDATE contestant
                     SET votes = votes + ?
                     WHERE normalized_contested_name = ?
                     AND identity_id = ?
                     AND network = ?",
                    params![vote_strength, contested_name, identity.to_vec(), network],
                )?;
            }
            ResourceVoteChoice::Abstain => {
                // Increment the abstain vote count in the contested_name table
                tx.execute(
                    "UPDATE contested_name
                     SET abstain_votes = abstain_votes + ?
                     WHERE normalized_contested_name = ? AND network = ?",
                    params![vote_strength, contested_name, network],
                )?;
            }
            ResourceVoteChoice::Lock => {
                // Increment the locked vote count in the contested_name table
                tx.execute(
                    "UPDATE contested_name
                     SET locked_votes = locked_votes + ?
                     WHERE normalized_contested_name = ? AND network = ?",
                    params![vote_strength, contested_name, network],
                )?;
            }
        }

        // Commit the transaction
        if let Err(e) = tx.commit() {
            error!("Failed to commit transaction: {:?}", e);
            return Err(e);
        }

        info!("Vote tally updated successfully for '{}'", contested_name);
        Ok(())
    }
}
