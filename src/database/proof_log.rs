use crate::database::Database;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use rusqlite::params;
use std::ops::Range;

impl Database {
    pub fn initialize_proof_log_table(&self) -> rusqlite::Result<()> {
        // Create the proof log tree
        self.execute(
            "CREATE TABLE IF NOT EXISTS proof_log (
                        proof_id INTEGER PRIMARY KEY AUTOINCREMENT,
                        request_type INTEGER NOT NULL,
                        request_bytes BLOB NOT NULL,
                        height INTEGER NOT NULL,
                        time_ms INTEGER NOT NULL,
                        proof_bytes BLOB NOT NULL,
                        error TEXT
                    )",
            [],
        )?;

        // Create an index on request_type and time for combined queries
        self.execute(
    "CREATE INDEX IF NOT EXISTS idx_proof_log_request_type_time ON proof_log (request_type, time_ms)",
    [],
    )?;

        // Create an index on time for queries ordered by time
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_proof_log_time ON proof_log (time_ms)",
            [],
        )?;

        // Index for error, request_type, and time
        self.execute(
    "CREATE INDEX IF NOT EXISTS idx_proof_log_error_request_type_time ON proof_log (error, request_type, time_ms)",
    [],
    )?;

        // Index for error and time
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_proof_log_error_time ON proof_log (error, time_ms)",
            [],
        )?;
        Ok(())
    }

    /// Inserts a new ProofLogItem into the proof_log table
    pub fn insert_proof_log_item(&self, item: ProofLogItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Convert RequestType to u8
        let request_type_int: u8 = item.request_type.into();

        conn.execute(
            "INSERT INTO proof_log (request_type, request_bytes, height, time_ms, proof_bytes, error)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                request_type_int,
                item.request_bytes,
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
        let conn = self.conn.lock().unwrap();

        // Build the query based on the only_get_errored flag
        let mut query = String::from(
            "SELECT request_type, request_bytes, height, time_ms, proof_bytes, error FROM proof_log",
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
                let height: u64 = row.get(2)?;
                let time_ms: u64 = row.get(3)?;
                let proof_bytes: Vec<u8> = row.get(4)?;
                let error: Option<String> = row.get(5)?;

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
