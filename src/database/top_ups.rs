use crate::database::Database;
use crate::lock_helper::MutexExt;
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
        let conn = self.conn.lock_or_recover();
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
        amount: u64,
    ) -> rusqlite::Result<()> {
        self.execute(
            "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES (?, ?, ?)",
            params![identity_id, top_up_index, amount],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;

    fn insert_test_identity(db: &crate::database::Database, id: &[u8]) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO identity (id, is_local, network) VALUES (?, 1, 'testnet')",
            rusqlite::params![id],
        )
        .expect("Failed to insert test identity");
    }

    #[test]
    fn test_insert_and_get_next_index() {
        let db = create_test_database().expect("Failed to create test database");
        let identity_id: [u8; 32] = [1u8; 32];
        insert_test_identity(&db, &identity_id);

        // Insert first top-up at index 0
        db.insert_top_up(&identity_id, 0, 1_000_000).unwrap();
        let next = db.get_next_top_up_index(&identity_id).unwrap();
        assert_eq!(next, 1);

        // Insert second top-up
        db.insert_top_up(&identity_id, 1, 2_000_000).unwrap();
        let next = db.get_next_top_up_index(&identity_id).unwrap();
        assert_eq!(next, 2);
    }

    #[test]
    fn test_multiple_identities_independent() {
        let db = create_test_database().expect("Failed to create test database");
        let id1: [u8; 32] = [1u8; 32];
        let id2: [u8; 32] = [2u8; 32];
        insert_test_identity(&db, &id1);
        insert_test_identity(&db, &id2);

        db.insert_top_up(&id1, 0, 100).unwrap();
        db.insert_top_up(&id1, 1, 200).unwrap();
        db.insert_top_up(&id2, 0, 300).unwrap();

        assert_eq!(db.get_next_top_up_index(&id1).unwrap(), 2);
        assert_eq!(db.get_next_top_up_index(&id2).unwrap(), 1);
    }

    #[test]
    fn test_large_amounts() {
        let db = create_test_database().expect("Failed to create test database");
        let id: [u8; 32] = [1u8; 32];
        insert_test_identity(&db, &id);

        let large_amount: u64 = 10_000_000_000_000; // 100,000 DASH in duffs
        db.insert_top_up(&id, 0, large_amount).unwrap();

        // Verify by reading back
        let conn = db.conn.lock().unwrap();
        let amount: u64 = conn
            .query_row(
                "SELECT amount FROM top_up WHERE identity_id = ? AND top_up_index = 0",
                rusqlite::params![id.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(amount, large_amount);
    }

    #[test]
    fn test_duplicate_index_fails() {
        let db = create_test_database().expect("Failed to create test database");
        let id: [u8; 32] = [1u8; 32];
        insert_test_identity(&db, &id);

        db.insert_top_up(&id, 0, 100).unwrap();
        // Inserting same (identity_id, top_up_index) should fail due to PRIMARY KEY constraint
        let result = db.insert_top_up(&id, 0, 200);
        assert!(result.is_err());
    }
}
