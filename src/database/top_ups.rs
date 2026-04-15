use crate::database::Database;
use rusqlite::params;

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

    /// Record a top-up as the next row for this identity. The `top_up_index`
    /// is assigned atomically in SQL as `MAX(existing) + 1` (0 if none), so
    /// concurrent callers cannot collide on the PK. This is a pure local
    /// history/ordering counter — it is NOT the on-chain derivation index
    /// (that is owned by dashcore's `AddressPool`).
    pub fn insert_next_top_up(&self, identity_id: &[u8], amount: u64) -> rusqlite::Result<u32> {
        let conn = self.conn.lock().unwrap();
        let next_index: u32 = conn.query_row(
            "INSERT INTO top_up (identity_id, top_up_index, amount)
             VALUES (
                 ?1,
                 COALESCE((SELECT MAX(top_up_index) FROM top_up WHERE identity_id = ?1), -1) + 1,
                 ?2
             )
             RETURNING top_up_index",
            params![identity_id, amount],
            |row| row.get(0),
        )?;
        Ok(next_index)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use rusqlite::params;

    /// Insert a minimal identity row so the top_up FK is satisfied.
    fn insert_identity_stub(db: &crate::database::Database, id: &[u8]) {
        db.execute(
            "INSERT INTO identity (id, data, is_local, network) VALUES (?, ?, 1, 'testnet')",
            params![id, Vec::<u8>::new()],
        )
        .expect("insert identity stub");
    }

    #[test]
    fn test_insert_next_top_up_assigns_sequential_indices() {
        let db = create_test_database().unwrap();
        let identity_id = [0xAB; 32];
        insert_identity_stub(&db, &identity_id);

        // First insert -> index 0
        let i0 = db.insert_next_top_up(&identity_id, 100).unwrap();
        assert_eq!(i0, 0);

        // Second insert -> index 1
        let i1 = db.insert_next_top_up(&identity_id, 200).unwrap();
        assert_eq!(i1, 1);

        // Third insert -> index 2 (proves no off-by-one when MAX is non-zero)
        let i2 = db.insert_next_top_up(&identity_id, 300).unwrap();
        assert_eq!(i2, 2);
    }

    #[test]
    fn test_insert_next_top_up_is_per_identity() {
        let db = create_test_database().unwrap();
        let id_a = [0xAA; 32];
        let id_b = [0xBB; 32];
        insert_identity_stub(&db, &id_a);
        insert_identity_stub(&db, &id_b);

        // Each identity has its own counter starting at 0.
        assert_eq!(db.insert_next_top_up(&id_a, 1).unwrap(), 0);
        assert_eq!(db.insert_next_top_up(&id_a, 2).unwrap(), 1);
        assert_eq!(db.insert_next_top_up(&id_b, 99).unwrap(), 0);
        assert_eq!(db.insert_next_top_up(&id_a, 3).unwrap(), 2);
        assert_eq!(db.insert_next_top_up(&id_b, 88).unwrap(), 1);
    }
}
