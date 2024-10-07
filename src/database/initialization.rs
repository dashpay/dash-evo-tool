use crate::database::Database;

impl Database {
    pub fn initialize(&self) -> rusqlite::Result<()> {
        // Create the identities table
        self.execute(
            "CREATE TABLE IF NOT EXISTS identity (
                id BLOB PRIMARY KEY,
                data BLOB,
                is_local INTEGER NOT NULL,
                alias TEXT,
                info TEXT,
                identity_type TEXT,
                network TEXT NOT NULL
            )",
            [],
        )?;

        // Create the contested names table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contested_name (
                normalized_contested_name TEXT,
                locked_votes INTEGER,
                abstain_votes INTEGER,
                awarded_to BLOB,
                ending_time INTEGER,
                network TEXT NOT NULL,
                PRIMARY KEY (normalized_contested_name, network)
            )",
            [],
        )?;

        // Create the contestants table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contestant (
                contest_id TEXT,
                identity_id BLOB,
                name TEXT,
                votes INTEGER,
                network TEXT NOT NULL,
                PRIMARY KEY (contest_id, identity_id, network),
                FOREIGN KEY (contest_id) REFERENCES contested_names(contest_id),
                FOREIGN KEY (identity_id) REFERENCES identities(id)
            )",
            [],
        )?;

        // Create the contracts table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contract (
                contract_id BLOB,
                contract BLOB,
                name TEXT,
                network TEXT NOT NULL,
                PRIMARY KEY (contract_id, network)
            )",
            [],
        )?;

        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_name_network ON contract (name, network)",
            [],
        )?;

        Ok(())
    }
}
