use dash_sdk::platform::Identifier;
use rusqlite::params;

use super::Database;

impl Database {
    /// Creates the identity_order table if it doesn't already exist
    /// with two columns: `pos` (int) and `identity_id` (blob).
    /// pos is the "position" in the custom ordering.
    fn ensure_token_order_table_exists(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS token_order (
                pos INTEGER NOT NULL,
                token_id BLOB NOT NULL,
                PRIMARY KEY(pos)
             )",
        )?;
        Ok(())
    }

    /// Saves the user’s custom identity order (the entire list).
    /// This method overwrites whatever was there before.
    pub fn save_token_order(&self, all_ids: Vec<Identifier>) -> rusqlite::Result<()> {
        // Make sure table exists
        self.ensure_token_order_table_exists()?;

        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Clear existing rows
        tx.execute("DELETE FROM token_order", [])?;

        // Insert each ID with a numeric pos = 0..N
        for (pos, id) in all_ids.iter().enumerate() {
            let id_bytes = id.to_vec();
            tx.execute(
                "INSERT INTO token_order (pos, token_id)
                 VALUES (?1, ?2)",
                params![pos as i64, id_bytes],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Loads the custom identity order from the DB, returning a list of Identifiers in the stored order.
    /// If there's no data, returns an empty Vec.
    pub fn load_token_order(&self) -> rusqlite::Result<Vec<Identifier>> {
        // Make sure table exists (in case it doesn't)
        self.ensure_token_order_table_exists()?;

        let conn = self.conn.lock().unwrap();

        // Read all rows sorted by pos
        let mut stmt = conn.prepare(
            "SELECT token_id FROM token_order
             ORDER BY pos ASC",
        )?;

        let mut rows = stmt.query([])?;
        let mut result = Vec::new();

        while let Some(row) = rows.next()? {
            let id_bytes: Vec<u8> = row.get(0)?;
            // Convert from raw bytes to an Identifier
            if let Ok(identifier) = Identifier::from_vec(id_bytes) {
                result.push(identifier);
            } else {
                // If for some reason it fails to parse, skip it or handle error
            }
        }

        Ok(result)
    }
}
