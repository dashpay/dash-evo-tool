use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use crate::model::wallet::{Wallet, WalletId};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

impl Database {
    /// Updates the alias of a specified identity.
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let conn = self.conn.lock().unwrap();

        let rows_updated = conn.execute(
            "UPDATE identity SET alias = ? WHERE id = ?",
            params![new_alias, id],
        )?;

        if rows_updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        Ok(())
    }

    pub fn get_identity_alias(&self, identifier: &Identifier) -> rusqlite::Result<Option<String>> {
        let id = identifier.to_vec();
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare("SELECT alias FROM identity WHERE id = ?")?;
        let alias: Option<String> = stmt.query_row(params![id], |row| row.get(0)).ok();

        Ok(alias)
    }

    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletId, u32)>,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        let network = app_context.network.to_string();

        let status = qualified_identity.status.as_u8();

        if let Some((wallet, wallet_index)) = wallet_and_identity_id_info {
            // If wallet information is provided, insert with wallet and wallet_index
            self.execute(
                "INSERT OR REPLACE INTO identity
             (id, data, is_local, alias, identity_type, network, wallet, wallet_index, status)
             VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    data,
                    alias,
                    identity_type,
                    network,
                    wallet,
                    wallet_index,
                    status,
                ],
            )?;
        } else {
            tracing::warn!(identity_id=?id, alias, network, "saving identity without wallet; this needs investigating");
            // If wallet information is not provided, insert without wallet and wallet_index
            self.execute(
                "INSERT OR REPLACE INTO identity
             (id, data, is_local, alias, identity_type, network, status)
             VALUES (?, ?, 1, ?, ?, ?, ?)",
                params![id, data, alias, identity_type, network, status],
            )?;
        }

        Ok(())
    }

    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        // Extract the fields from `qualified_identity` to use in the SQL update
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        // Get the network string from the app context
        let network = app_context.network.to_string();

        let status = qualified_identity.status.as_u8();

        // Execute the update statement
        self.execute(
            "UPDATE identity
         SET data = ?, alias = ?, identity_type = ?, network = ?, is_local = 1, status = ?
         WHERE id = ?",
            params![data, alias, identity_type, network, status, id],
        )?;

        Ok(())
    }

    /// Returns all local identities for the current network.
    ///
    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    pub fn get_local_qualified_identities(
        &self,
        app_context: &AppContext,
        wallets: &BTreeMap<WalletId, Arc<RwLock<Wallet>>>,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Use a LEFT JOIN to load identities and their top-ups in a single query,
        // avoiding the N+1 query pattern of querying top_up per identity.
        let mut stmt = conn.prepare(
            "SELECT i.data, i.alias, i.wallet_index, i.status, i.id, t.top_up_index, t.amount
             FROM identity i
             LEFT JOIN top_up t ON i.id = t.identity_id
             WHERE i.is_local = 1 AND i.network = ? AND i.data IS NOT NULL
             ORDER BY i.id",
        )?;

        let mut rows = stmt.query(params![network])?;

        let mut identities: Vec<QualifiedIdentity> = Vec::new();
        let mut current_id: Option<Vec<u8>> = None;

        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(4)?;

            if current_id.as_ref() != Some(&id) {
                // New identity row
                let data: Vec<u8> = row.get(0)?;
                let alias: Option<String> = row.get(1)?;
                let wallet_index: Option<u32> = row.get(2)?;
                let status: Option<u8> = row.get(3)?;

                let mut identity =
                    QualifiedIdentity::from_bytes(&data).map_err(super::CorruptedBlobError)?;
                identity.alias = alias;
                identity.wallet_index = wallet_index;
                identity.status = IdentityStatus::from_u8(status.unwrap_or(2));
                identity.network = app_context.network;
                identity.associated_wallets = wallets.clone();
                identity.top_ups = BTreeMap::new();

                identities.push(identity);
                current_id = Some(id);
            }

            // Add top-up entry if present (NULL when identity has no top-ups)
            let top_up_index: Option<u32> = row.get(5)?;
            let amount: Option<u64> = row.get(6)?;
            if let (Some(idx), Some(amt)) = (top_up_index, amount)
                && let Some(identity) = identities.last_mut()
            {
                identity.top_ups.insert(idx, amt);
            }
        }

        Ok(identities)
    }

    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    #[allow(dead_code)] // May be used for filtering identities that belong to specific wallets
    pub fn get_local_qualified_identities_in_wallets(
        &self,
        app_context: &AppContext,
        wallets: &BTreeMap<WalletId, Arc<RwLock<Wallet>>>,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Use a LEFT JOIN to load identities and their top-ups in a single query.
        let mut stmt = conn.prepare(
            "SELECT i.data, i.alias, i.wallet_index, i.status, i.id, t.top_up_index, t.amount
             FROM identity i
             LEFT JOIN top_up t ON i.id = t.identity_id
             WHERE i.is_local = 1 AND i.network = ? AND i.data IS NOT NULL AND i.wallet_index IS NOT NULL
             ORDER BY i.id",
        )?;

        let mut rows = stmt.query(params![network])?;

        let mut identities: Vec<QualifiedIdentity> = Vec::new();
        let mut current_id: Option<Vec<u8>> = None;

        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(4)?;

            if current_id.as_ref() != Some(&id) {
                // New identity row
                let data: Vec<u8> = row.get(0)?;
                let alias: Option<String> = row.get(1)?;
                let wallet_index: Option<u32> = row.get(2)?;
                let status: Option<u8> = row.get(3)?;

                let mut identity =
                    QualifiedIdentity::from_bytes(&data).map_err(super::CorruptedBlobError)?;
                identity.alias = alias;
                identity.wallet_index = wallet_index;
                identity.status = IdentityStatus::from_u8(status.unwrap_or(2));
                identity.network = app_context.network;
                identity.associated_wallets = wallets.clone();
                identity.top_ups = BTreeMap::new();

                identities.push(identity);
                current_id = Some(id);
            }

            // Add top-up entry if present
            let top_up_index: Option<u32> = row.get(5)?;
            let amount: Option<u64> = row.get(6)?;
            if let (Some(idx), Some(amt)) = (top_up_index, amount)
                && let Some(identity) = identities.last_mut()
            {
                identity.top_ups.insert(idx, amt);
            }
        }

        Ok(identities)
    }

    /// Returns an error if the stored identity blob is corrupted.
    /// This is intentional — identities hold private keys and balance data,
    /// so ignoring corruption could cause loss of funds.
    pub fn get_identity_by_id(
        &self,
        identifier: &Identifier,
        app_context: &AppContext,
        wallets: &BTreeMap<WalletId, Arc<RwLock<Wallet>>>,
    ) -> rusqlite::Result<Option<QualifiedIdentity>> {
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Use a LEFT JOIN to load identity and its top-ups in a single query.
        let mut stmt = conn.prepare(
            "SELECT i.data, i.alias, i.wallet_index, i.status, t.top_up_index, t.amount
             FROM identity i
             LEFT JOIN top_up t ON i.id = t.identity_id
             WHERE i.id = ? AND i.is_local = 1 AND i.network = ? AND i.data IS NOT NULL",
        )?;

        let mut rows = stmt.query(params![identifier.to_buffer(), network])?;

        let mut identity: Option<QualifiedIdentity> = None;

        while let Some(row) = rows.next()? {
            if identity.is_none() {
                let data: Vec<u8> = row.get(0)?;
                let alias: Option<String> = row.get(1)?;
                let wallet_index: Option<u32> = row.get(2)?;
                let status: Option<u8> = row.get(3)?;

                let mut qi =
                    QualifiedIdentity::from_bytes(&data).map_err(super::CorruptedBlobError)?;
                qi.alias = alias;
                qi.wallet_index = wallet_index;
                qi.status = IdentityStatus::from_u8(status.unwrap_or(2));
                qi.network = app_context.network;
                qi.associated_wallets = wallets.clone();
                qi.top_ups = BTreeMap::new();

                identity = Some(qi);
            }

            // Add top-up entry if present
            let top_up_index: Option<u32> = row.get(4)?;
            let amount: Option<u64> = row.get(5)?;
            if let (Some(idx), Some(amt)) = (top_up_index, amount)
                && let Some(ref mut qi) = identity
            {
                qi.top_ups.insert(idx, amt);
            }
        }

        Ok(identity)
    }

    /// Returns the set of identity wallet indices already used by the given wallet.
    ///
    /// This queries the `identity` table for all rows belonging to the wallet
    /// (identified by seed hash) and returns the `wallet_index` values as a set.
    pub fn get_wallet_identity_indices(
        &self,
        wallet_seed_hash: &WalletId,
        network: Network,
    ) -> HashSet<u32> {
        let network_str = network.to_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT wallet_index FROM identity \
                 WHERE wallet = ?1 AND network = ?2 AND wallet_index IS NOT NULL",
            )
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to prepare wallet identity indices query: {e}");
                // Return an empty set below via early return isn't possible here,
                // but the query is simple enough that prepare shouldn't fail.
                panic!("Failed to prepare wallet identity indices query: {e}");
            });
        stmt.query_map(
            rusqlite::params![wallet_seed_hash.as_slice(), network_str],
            |row| row.get::<_, u32>(0),
        )
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    pub fn get_local_voting_identities(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM identity WHERE is_local = 1 AND network = ? AND identity_type != 'User' AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let mut identity =
                QualifiedIdentity::from_bytes(&data).map_err(super::CorruptedBlobError)?;
            identity.network = app_context.network;

            Ok(identity)
        })?;

        let identities: rusqlite::Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }

    /// Retrieves all local user identities along with their associated wallet IDs.
    ///
    /// Stops on the first corrupted identity blob and returns an error.
    /// This is intentional — identities hold private keys and balance data,
    /// so skipping a corrupted entry could cause loss of funds.
    ///
    /// Caller should insert wallet references into associated_wallets before using the identities.
    #[allow(clippy::let_and_return)]
    pub fn get_local_user_identities(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<(QualifiedIdentity, Option<WalletId>)>> {
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data,wallet FROM identity WHERE is_local = 1 AND network = ? AND identity_type = 'User' AND data IS NOT NULL",
        )?;
        let identities: Result<Vec<(QualifiedIdentity, Option<WalletId>)>, rusqlite::Error> =
            stmt.query_map(params![network], |row| {
                let data: Vec<u8> = row.get(0)?;
                let wallet_id: Option<WalletId> = row.get(1)?;
                let mut identity =
                    QualifiedIdentity::from_bytes(&data).map_err(super::CorruptedBlobError)?;
                identity.network = app_context.network;

                Ok((identity, wallet_id))
            })?
            .collect();

        identities
    }

    /// Deletes a local qualified identity with the given identifier from the database.
    pub fn delete_local_qualified_identity(
        &self,
        identifier: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Perform the deletion only if the identity is marked as local
        conn.execute(
            "DELETE FROM identity WHERE id = ? AND network = ? AND is_local = 1",
            params![id, network],
        )?;

        Ok(())
    }

    /// Deletes all local qualified identities in Devnet variants and Regtest.
    pub fn delete_all_identities_in_all_devnets_and_regtest(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM identity WHERE (network LIKE 'devnet%' OR network = 'regtest')",
            [],
        )?;

        Ok(())
    }

    /// Deletes a local qualified identity with the given identifier from the database.
    pub fn delete_all_local_qualified_identities_in_devnet(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        if app_context.network != Network::Devnet {
            return Ok(());
        }
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Perform the deletion only if the identity is marked as local
        conn.execute(
            "DELETE FROM identity WHERE network = ? AND is_local = 1",
            params![network],
        )?;

        Ok(())
    }

    /// Creates the identity_order table if it doesn't already exist
    /// with two columns: `pos` (int) and `identity_id` (blob).
    /// pos is the "position" in the custom ordering.
    pub fn initialize_identity_order_table(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS identity_order (
            pos INTEGER NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos),
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        )",
            [],
        )?;

        Ok(())
    }

    /// Saves the user’s custom identity order (the entire list).
    /// This method overwrites whatever was there before.
    pub fn save_identity_order(&self, all_ids: Vec<Identifier>) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Clear existing rows
        tx.execute("DELETE FROM identity_order", [])?;

        // Insert each ID with a numeric pos = 0..N
        for (pos, id) in all_ids.iter().enumerate() {
            let id_bytes = id.to_vec();
            tx.execute(
                "INSERT INTO identity_order (pos, identity_id)
                 VALUES (?1, ?2)",
                params![pos as i64, id_bytes],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Loads the user's custom identity order (the entire list).
    /// If an identity in the order doesn't exist in the identity table, it is removed.
    pub fn load_identity_order(&self) -> rusqlite::Result<Vec<Identifier>> {
        let conn = self.conn.lock().unwrap();

        // Use a LEFT JOIN to get all order entries and detect dangling references
        // in a single query instead of per-row EXISTS checks.
        let mut stmt = conn.prepare(
            "SELECT io.identity_id, i.id IS NOT NULL AS exists_in_identity
             FROM identity_order io
             LEFT JOIN identity i ON io.identity_id = i.id
             ORDER BY io.pos ASC",
        )?;

        let mut rows = stmt.query([])?;
        let mut final_list = Vec::new();
        let mut to_remove = Vec::new();

        while let Some(row) = rows.next()? {
            let id_bytes: Vec<u8> = row.get(0)?;
            let exists: bool = row.get(1)?;

            let identifier = match Identifier::from_vec(id_bytes.clone()) {
                Ok(id) => id,
                Err(_) => {
                    // If parsing as an Identifier fails, queue for removal
                    to_remove.push(id_bytes);
                    continue;
                }
            };

            if exists {
                final_list.push(identifier);
            } else {
                // Queue for removal because it doesn't exist in the identity table
                to_remove.push(identifier.to_vec());
            }
        }

        // Remove any "dangling" references
        for id in to_remove {
            conn.execute(
                "DELETE FROM identity_order WHERE identity_id = ?",
                params![id],
            )?;
        }

        Ok(final_list)
    }

    /// Fixes bug in identity table where network name for devnet was stored as `devnet:` instead of `devnet`.
    pub fn fix_identity_devnet_network_name(&self, conn: &Connection) -> rusqlite::Result<()> {
        const TABLES: [&str; 11] = [
            "asset_lock_transaction",
            "contestant",
            "contested_name",
            "contract",
            "identity",
            "identity_token_balances",
            "scheduled_votes",
            "settings",
            "token",
            "utxos",
            "wallet",
        ];

        for t in TABLES {
            conn.execute(
                &format!(
                    "UPDATE {} SET network = 'devnet' WHERE network = 'devnet:'",
                    t
                ),
                [],
            )?;

            conn.execute(
                &format!(
                    "UPDATE {} SET network = 'regtest' WHERE network = 'local'",
                    t
                ),
                [],
            )?;
        }

        tracing::debug!("Updated network names in database");

        Ok(())
    }

    pub fn rename_identity_column_is_in_creation_to_status(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        // Rename the column in the identity table
        conn.execute(
            "ALTER TABLE identity RENAME COLUMN is_in_creation TO status",
            [],
        )?;

        // Update the status values to match the new enum
        conn.execute(
            "UPDATE identity SET status = 2 WHERE status = 0", // Active was 0, now it's 2
            [],
        )?;
        tracing::debug!("Renamed column 'is_in_creation' to 'status' in identity table");

        Ok(())
    }
}
