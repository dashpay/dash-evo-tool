use crate::lock_helper::MutexExt;
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
    pub fn initialize_scheduled_votes_table(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        // Create the scheduled_votes table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS scheduled_votes (
                identity_id BLOB NOT NULL,
                contested_name TEXT NOT NULL,
                vote_choice TEXT NOT NULL,
                time INTEGER NOT NULL,
                executed INTEGER NOT NULL DEFAULT 0,
                network TEXT NOT NULL,
                PRIMARY KEY (identity_id, contested_name),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_scheduled_votes_network ON scheduled_votes (network)",
            [],
        )?;
        Ok(())
    }

    pub fn update_scheduled_votes_table(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        {
            // Check if the foreign key already exists
            let mut stmt = conn.prepare("PRAGMA foreign_key_list('scheduled_votes')")?;
            let fk_exists = stmt
                .query_map([], |row| row.get::<_, String>(2))?
                .any(|table_name_result| table_name_result.ok().as_deref() == Some("identity"));

            if fk_exists {
                // Foreign key already exists, no need to alter the table
                return Ok(());
            }
        }

        // SQLite doesn't directly support adding foreign keys to existing tables.
        // We need to recreate the table.

        conn.execute("PRAGMA foreign_keys = OFF", [])?;

        // Rename existing table
        conn.execute(
            "ALTER TABLE scheduled_votes RENAME TO scheduled_votes_old",
            [],
        )?;

        // Create the new table with the foreign key constraint
        conn.execute(
            "CREATE TABLE scheduled_votes (
            identity_id BLOB NOT NULL,
            contested_name TEXT NOT NULL,
            vote_choice TEXT NOT NULL,
            time INTEGER NOT NULL,
            executed INTEGER NOT NULL DEFAULT 0,
            network TEXT NOT NULL,
            PRIMARY KEY (identity_id, contested_name),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        )",
            [],
        )?;

        // Copy data from old to new table
        conn.execute(
            "INSERT INTO scheduled_votes (identity_id, contested_name, vote_choice, time, executed, network)
         SELECT identity_id, contested_name, vote_choice, time, executed, network
         FROM scheduled_votes_old",
            [],
        )?;

        // Drop the old table
        conn.execute("DROP TABLE scheduled_votes_old", [])?;

        conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(())
    }

    pub fn insert_scheduled_votes(
        &self,
        app_context: &AppContext,
        votes: &Vec<ScheduledDPNSVote>,
    ) -> rusqlite::Result<()> {
        let network = app_context.network.to_string();
        let mut conn = self.conn.lock_or_recover();
        let tx = conn.transaction()?;
        for vote in votes {
            let vote_choice = vote.choice.to_string();
            tx.execute(
                "INSERT OR REPLACE INTO scheduled_votes (identity_id, contested_name, vote_choice, time, executed, network) VALUES (?, ?, ?, ?, 0, ?)",
                params![vote.voter_id.as_slice(), vote.contested_name, vote_choice, vote.unix_timestamp, network],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_scheduled_vote(
        &self,
        app_context: &AppContext,
        identity_id: &[u8],
        contested_name: &str,
    ) -> rusqlite::Result<()> {
        let network = app_context.network.to_string();
        let conn = self.conn.lock_or_recover();
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
        let network = app_context.network.to_string();
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
        let network = app_context.network.to_string();

        let conn = self.conn.lock_or_recover();
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
                                Identifier::from_string(&towards_id, Encoding::Base58).map_err(
                                    |e| {
                                        rusqlite::Error::FromSqlConversionFailure(
                                            0,
                                            rusqlite::types::Type::Blob,
                                            Box::new(e),
                                        )
                                    },
                                )?,
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
                voter_id: Identifier::from_bytes(&voter_id_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?,
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
        let network = app_context.network.to_string();
        let conn = self.conn.lock_or_recover();

        conn.execute(
            "DELETE FROM scheduled_votes WHERE network = ?",
            params![network],
        )?;

        Ok(())
    }

    pub fn clear_executed_scheduled_votes(&self, app_context: &AppContext) -> rusqlite::Result<()> {
        let network = app_context.network.to_string();
        let conn = self.conn.lock_or_recover();

        conn.execute(
            "DELETE FROM scheduled_votes WHERE executed = 1 AND network = ?",
            params![network],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use dash_sdk::platform::Identifier;

    fn insert_test_identity(db: &crate::database::Database, id: &Identifier) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO identity (id, is_local, network) VALUES (?, 1, 'testnet')",
            rusqlite::params![id.to_vec()],
        )
        .unwrap();
    }

    fn insert_raw_vote(
        db: &crate::database::Database,
        identity_id: &Identifier,
        name: &str,
        choice: &str,
        time: u64,
        executed: bool,
        network: &str,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO scheduled_votes (identity_id, contested_name, vote_choice, time, executed, network) VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![identity_id.to_vec(), name, choice, time, executed as i32, network],
        ).unwrap();
    }

    fn count_votes(db: &crate::database::Database, network: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM scheduled_votes WHERE network = ?",
            rusqlite::params![network],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn count_executed_votes(db: &crate::database::Database, network: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM scheduled_votes WHERE executed = 1 AND network = ?",
            rusqlite::params![network],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn test_insert_and_query_scheduled_votes_raw() {
        let db = create_test_database().unwrap();
        let voter = Identifier::random();
        insert_test_identity(&db, &voter);

        insert_raw_vote(&db, &voter, "dash-name", "Abstain", 1000, false, "testnet");
        insert_raw_vote(&db, &voter, "other-name", "Lock", 2000, false, "testnet");

        assert_eq!(count_votes(&db, "testnet"), 2);
    }

    #[test]
    fn test_vote_upsert_replaces() {
        let db = create_test_database().unwrap();
        let voter = Identifier::random();
        insert_test_identity(&db, &voter);

        insert_raw_vote(&db, &voter, "dash-name", "Abstain", 1000, false, "testnet");
        insert_raw_vote(&db, &voter, "dash-name", "Lock", 2000, false, "testnet");

        // Should be 1 vote (same PK), with updated choice
        assert_eq!(count_votes(&db, "testnet"), 1);

        let conn = db.conn.lock().unwrap();
        let choice: String = conn.query_row(
            "SELECT vote_choice FROM scheduled_votes WHERE identity_id = ? AND contested_name = ?",
            rusqlite::params![voter.to_vec(), "dash-name"],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(choice, "Lock");
    }

    #[test]
    fn test_mark_executed_and_clear_executed() {
        let db = create_test_database().unwrap();
        let voter = Identifier::random();
        insert_test_identity(&db, &voter);

        insert_raw_vote(&db, &voter, "name-1", "Abstain", 1000, false, "testnet");
        insert_raw_vote(&db, &voter, "name-2", "Lock", 2000, true, "testnet");

        assert_eq!(count_executed_votes(&db, "testnet"), 1);

        // Clear executed votes via raw SQL (simulating what clear_executed_scheduled_votes does)
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM scheduled_votes WHERE executed = 1 AND network = ?",
                rusqlite::params!["testnet"],
            )
            .unwrap();
        }

        assert_eq!(count_votes(&db, "testnet"), 1);
        assert_eq!(count_executed_votes(&db, "testnet"), 0);
    }

    #[test]
    fn test_delete_specific_vote() {
        let db = create_test_database().unwrap();
        let voter = Identifier::random();
        insert_test_identity(&db, &voter);

        insert_raw_vote(&db, &voter, "name-1", "Abstain", 1000, false, "testnet");
        insert_raw_vote(&db, &voter, "name-2", "Lock", 2000, false, "testnet");

        // Delete one specific vote
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM scheduled_votes WHERE identity_id = ? AND contested_name = ? AND network = ?",
                rusqlite::params![voter.to_vec(), "name-1", "testnet"],
            ).unwrap();
        }

        assert_eq!(count_votes(&db, "testnet"), 1);
    }

    #[test]
    fn test_network_field_stored_correctly() {
        let db = create_test_database().unwrap();
        let voter1 = Identifier::random();
        let voter2 = Identifier::random();
        insert_test_identity(&db, &voter1);
        insert_test_identity(&db, &voter2);

        // Use different voters since PK is (identity_id, contested_name) without network
        insert_raw_vote(&db, &voter1, "name-1", "Abstain", 1000, false, "testnet");
        insert_raw_vote(&db, &voter2, "name-2", "Lock", 2000, false, "dash");

        assert_eq!(count_votes(&db, "testnet"), 1);
        assert_eq!(count_votes(&db, "dash"), 1);

        // Clear testnet votes
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM scheduled_votes WHERE network = ?",
                rusqlite::params!["testnet"],
            )
            .unwrap();
        }

        assert_eq!(count_votes(&db, "testnet"), 0);
        assert_eq!(count_votes(&db, "dash"), 1);
    }
}
