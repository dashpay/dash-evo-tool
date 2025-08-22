use crate::database::Database;
use dash_sdk::dpp::balances::credits::Duffs;
use rusqlite::{OptionalExtension, params};

impl Database {
    pub fn initialize_top_up_table(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        // Create the top_up table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS top_up (
                identity_id BLOB NOT NULL,
                top_up_index INTEGER NOT NULL,
                amount INTEGER NOT NULL,
                PRIMARY KEY (identity_id, top_up_index),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    #[allow(dead_code)] // May be used for generating sequential top-up indices
    pub fn get_next_top_up_index(&self, identity_id: &[u8]) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let max_index: Option<u64> = conn
            .query_row(
                "SELECT MAX(top_up_index) FROM top_up WHERE identity_id = ?",
                params![identity_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(max_index.unwrap_or(0) + 1)
    }

    pub fn insert_top_up(
        &self,
        identity_id: &[u8],
        top_up_index: u32,
        amount: Duffs,
    ) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES (?, ?, ?)",
            params![identity_id, top_up_index, amount],
        )?;
        Ok(())
    }
}
