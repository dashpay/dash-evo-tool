use crate::database::Database;
use crate::lock_helper::MutexExt;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use rusqlite::params;
use std::ops::Range;

impl Database {
    #[allow(dead_code)] // May be used for database migrations or testing cleanup
    pub fn drop_proof_log_table(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        // Execute the SQL command to drop the proof_log table
        conn.execute("DROP TABLE IF EXISTS proof_log", [])?;
        Ok(())
    }

    pub fn initialize_proof_log_table(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        // Create the proof log tree
        conn.execute(
            "CREATE TABLE IF NOT EXISTS proof_log (
                        proof_id INTEGER PRIMARY KEY AUTOINCREMENT,
                        request_type INTEGER NOT NULL,
                        request_bytes BLOB NOT NULL,
                        path_query_bytes BLOB NOT NULL,
                        height INTEGER NOT NULL,
                        time_ms INTEGER NOT NULL,
                        proof_bytes BLOB NOT NULL,
                        error TEXT
                    )",
            [],
        )?;

        // Create an index on request_type and time for combined queries
        conn.execute(
    "CREATE INDEX IF NOT EXISTS idx_proof_log_request_type_time ON proof_log (request_type, time_ms)",
    [],
    )?;

        // Create an index on time for queries ordered by time
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_proof_log_time ON proof_log (time_ms)",
            [],
        )?;

        // Index for error, request_type, and time
        conn.execute(
    "CREATE INDEX IF NOT EXISTS idx_proof_log_error_request_type_time ON proof_log (error, request_type, time_ms)",
    [],
    )?;

        // Index for error and time
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_proof_log_error_time ON proof_log (error, time_ms)",
            [],
        )?;
        Ok(())
    }

    /// Inserts a new ProofLogItem into the proof_log table
    pub fn insert_proof_log_item(&self, item: ProofLogItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock_or_recover();

        // Convert RequestType to u8
        let request_type_int: u8 = item.request_type.into();

        conn.execute(
            "INSERT INTO proof_log (request_type, request_bytes, path_query_bytes, height, time_ms, proof_bytes, error)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                request_type_int,
                item.request_bytes,
                item.verification_path_query_bytes,
                item.height,
                item.time_ms,
                item.proof_bytes,
                item.error,
            ],
        )?;

        Ok(())
    }

    /// Retrieves ProofLogItems with options for filtering and pagination
    pub fn get_proof_log_items(
        &self,
        only_get_errored: bool,
        range: Range<u64>,
    ) -> rusqlite::Result<Vec<ProofLogItem>> {
        let conn = self.conn.lock_or_recover();

        // Build the query based on the only_get_errored flag
        let mut query = String::from(
            "SELECT request_type, request_bytes, path_query_bytes, height, time_ms, proof_bytes, error FROM proof_log",
        );

        if only_get_errored {
            query.push_str(" WHERE error IS NOT NULL");
        }

        query.push_str(" ORDER BY time_ms DESC LIMIT ? OFFSET ?");

        let mut stmt = conn.prepare(&query)?;

        let proof_log_iter =
            stmt.query_map(params![range.end - range.start, range.start], |row| {
                let request_type_int: u8 = row.get(0)?;
                let request_bytes: Vec<u8> = row.get(1)?;
                let verification_path_query_bytes: Vec<u8> = row.get(2)?;
                let height: u64 = row.get(3)?;
                let time_ms: u64 = row.get(4)?;
                let proof_bytes: Vec<u8> = row.get(5)?;
                let error: Option<String> = row.get(6)?;

                // Convert u8 to RequestType
                let request_type = RequestType::try_from(request_type_int).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        request_type_int as usize,
                        rusqlite::types::Type::Integer,
                        Box::new(std::fmt::Error),
                    )
                })?;

                Ok(ProofLogItem {
                    request_type,
                    request_bytes,
                    verification_path_query_bytes,
                    height,
                    time_ms,
                    proof_bytes,
                    error,
                })
            })?;

        // Collect the results into a vector
        let items: rusqlite::Result<Vec<ProofLogItem>> = proof_log_iter.collect();

        items
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use crate::model::proof_log_item::{ProofLogItem, RequestType};

    fn make_item(
        request_type: RequestType,
        height: u64,
        time_ms: u64,
        error: Option<&str>,
    ) -> ProofLogItem {
        ProofLogItem {
            request_type,
            request_bytes: vec![1, 2, 3],
            verification_path_query_bytes: vec![4, 5, 6],
            height,
            time_ms,
            proof_bytes: vec![7, 8, 9],
            error: error.map(String::from),
        }
    }

    #[test]
    fn test_insert_and_retrieve_proof_log_item() {
        let db = create_test_database().expect("Failed to create test database");

        let item = make_item(RequestType::GetIdentity, 100, 1000, None);
        db.insert_proof_log_item(item.clone())
            .expect("Failed to insert");

        let items = db
            .get_proof_log_items(false, 0..10)
            .expect("Failed to get items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0], item);
    }

    #[test]
    fn test_insert_multiple_and_ordering() {
        let db = create_test_database().expect("Failed to create test database");

        db.insert_proof_log_item(make_item(RequestType::GetIdentity, 100, 1000, None))
            .unwrap();
        db.insert_proof_log_item(make_item(RequestType::GetDocuments, 200, 3000, None))
            .unwrap();
        db.insert_proof_log_item(make_item(RequestType::GetDataContract, 150, 2000, None))
            .unwrap();

        let items = db.get_proof_log_items(false, 0..10).unwrap();
        assert_eq!(items.len(), 3);
        // Should be ordered by time_ms DESC
        assert_eq!(items[0].time_ms, 3000);
        assert_eq!(items[1].time_ms, 2000);
        assert_eq!(items[2].time_ms, 1000);
    }

    #[test]
    fn test_filter_errored_items() {
        let db = create_test_database().expect("Failed to create test database");

        db.insert_proof_log_item(make_item(RequestType::GetIdentity, 100, 1000, None))
            .unwrap();
        db.insert_proof_log_item(make_item(
            RequestType::GetDocuments,
            200,
            2000,
            Some("proof mismatch"),
        ))
        .unwrap();
        db.insert_proof_log_item(make_item(RequestType::GetDataContract, 300, 3000, None))
            .unwrap();

        let errored = db.get_proof_log_items(true, 0..10).unwrap();
        assert_eq!(errored.len(), 1);
        assert_eq!(errored[0].error.as_deref(), Some("proof mismatch"));
        assert_eq!(errored[0].request_type, RequestType::GetDocuments);

        let all = db.get_proof_log_items(false, 0..10).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_pagination() {
        let db = create_test_database().expect("Failed to create test database");

        for i in 0..5 {
            db.insert_proof_log_item(make_item(RequestType::GetIdentity, i, i * 100, None))
                .unwrap();
        }

        // Get first 2 items (highest time_ms first)
        let page1 = db.get_proof_log_items(false, 0..2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].time_ms, 400);
        assert_eq!(page1[1].time_ms, 300);

        // Get next 2 items
        let page2 = db.get_proof_log_items(false, 2..4).unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].time_ms, 200);
        assert_eq!(page2[1].time_ms, 100);

        // Get last page
        let page3 = db.get_proof_log_items(false, 4..6).unwrap();
        assert_eq!(page3.len(), 1);
        assert_eq!(page3[0].time_ms, 0);
    }

    #[test]
    fn test_empty_result() {
        let db = create_test_database().expect("Failed to create test database");

        let items = db.get_proof_log_items(false, 0..10).unwrap();
        assert!(items.is_empty());

        let errored = db.get_proof_log_items(true, 0..10).unwrap();
        assert!(errored.is_empty());
    }

    #[test]
    fn test_request_type_roundtrip() {
        let db = create_test_database().expect("Failed to create test database");

        let types = [
            RequestType::BroadcastStateTransition,
            RequestType::GetIdentity,
            RequestType::GetDocuments,
            RequestType::GetDataContract,
            RequestType::WaitForStateTransitionResult,
            RequestType::GetContestedResources,
            RequestType::GetCurrentQuorumsInfo,
        ];

        for (i, rt) in types.iter().enumerate() {
            db.insert_proof_log_item(make_item(*rt, i as u64, i as u64, None))
                .unwrap();
        }

        let items = db.get_proof_log_items(false, 0..100).unwrap();
        assert_eq!(items.len(), types.len());

        // Verify all request types survived the roundtrip (items are in reverse order)
        for item in &items {
            assert!(types.contains(&item.request_type));
        }
    }
}
