use crate::database::Database;

pub const MIN_SUPPORTED_DB_VERSION: u16 = 0;

impl Database {
    pub fn initialize(&self) -> rusqlite::Result<()> {
        // Create the settings table
        self.execute(
            "CREATE TABLE IF NOT EXISTS settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            network TEXT NOT NULL,
            start_root_screen INTEGER NOT NULL,
            database_version INTEGER NOT NULL
        )",
            [],
        )?;

        // Create the wallet table
        self.execute(
            "CREATE TABLE IF NOT EXISTS wallet (
        seed_hash BLOB NOT NULL PRIMARY KEY,
        encrypted_seed BLOB NOT NULL,
        salt BLOB NOT NULL,
        nonce BLOB NOT NULL,
        master_ecdsa_bip44_account_0_epk BLOB NOT NULL,
        alias TEXT,
        is_main INTEGER,
        uses_password INTEGER NOT NULL,
        password_hint TEXT,
        network TEXT NOT NULL
    )",
            [],
        )?;

        // Create wallet addresses
        self.execute(
            "CREATE TABLE IF NOT EXISTS wallet_addresses (
        seed_hash BLOB NOT NULL,
        address TEXT NOT NULL,
        derivation_path TEXT NOT NULL,
        balance INTEGER,
        path_reference INTEGER NOT NULL,
        path_type INTEGER NOT NULL,
        PRIMARY KEY (seed_hash, address),
        FOREIGN KEY (seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
    )",
            [],
        )?;

        // Create the utxos table
        self.execute(
            "CREATE TABLE IF NOT EXISTS utxos (
                txid BLOB NOT NULL,
                vout INTEGER NOT NULL,
                address TEXT NOT NULL,
                value INTEGER NOT NULL,
                script_pubkey BLOB NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY (txid, vout, network)
            );",
            [],
        )?;

        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_reference ON wallet_addresses (path_reference)", [])?;
        self.execute("CREATE INDEX IF NOT EXISTS idx_wallet_addresses_path_type ON wallet_addresses (path_type)", [])?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_address ON utxos (address)",
            [],
        )?;
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_utxos_network ON utxos (network)",
            [],
        )?;

        self.execute(
            "CREATE TABLE IF NOT EXISTS asset_lock_transaction (
                tx_id TEXT PRIMARY KEY,
                transaction_data BLOB NOT NULL,
                amount INTEGER,
                instant_lock_data BLOB,
                chain_locked_height INTEGER,
                identity_id BLOB,
                identity_id_potentially_in_creation BLOB,
                wallet BLOB NOT NULL,
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE,
                FOREIGN KEY (identity_id_potentially_in_creation) REFERENCES identity(id),
                FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create the identities table
        self.execute(
            "CREATE TABLE IF NOT EXISTS identity (
                id BLOB PRIMARY KEY,
                data BLOB,
                is_in_creation INTEGER NOT NULL DEFAULT 0,
                is_local INTEGER NOT NULL,
                alias TEXT,
                info TEXT,
                wallet BLOB,
                wallet_index INTEGER,
                identity_type TEXT,
                network TEXT NOT NULL,
                CHECK ((wallet IS NOT NULL AND wallet_index IS NOT NULL) OR (wallet IS NULL AND wallet_index IS NULL)),
                FOREIGN KEY (wallet) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        // Create the composite index for faster querying
        self.execute(
            "CREATE INDEX IF NOT EXISTS idx_identity_local_network_type
     ON identity (is_local, network, identity_type)",
            [],
        )?;

        // Create the contested names table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contested_name (
                normalized_contested_name TEXT NOT NULL,
                locked_votes INTEGER,
                abstain_votes INTEGER,
                awarded_to BLOB,
                end_time INTEGER,
                locked INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER,
                network TEXT NOT NULL,
                PRIMARY KEY (normalized_contested_name, network)
            )",
            [],
        )?;

        // Create the contestants table
        self.execute(
            "CREATE TABLE IF NOT EXISTS contestant (
                normalized_contested_name TEXT NOT NULL,
                identity_id BLOB NOT NULL,
                name TEXT,
                votes INTEGER,
                created_at INTEGER,
                created_at_block_height INTEGER,
                created_at_core_block_height INTEGER,
                document_id BLOB,
                network TEXT NOT NULL,
                PRIMARY KEY (normalized_contested_name, identity_id, network),
                FOREIGN KEY (normalized_contested_name, network) REFERENCES contested_name(normalized_contested_name, network) ON DELETE CASCADE
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
