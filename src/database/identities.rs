use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use rusqlite::params;
use std::sync::{Arc, RwLock, RwLockReadGuard};

impl Database {
    /// Updates the alias of a specified identity.
    pub fn set_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE identity SET alias = ? WHERE id = ?",
            params![new_alias, id],
        )?;

        Ok(())
    }
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: Option<(&[u8], u32)>,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        let network = app_context.network_string();

        if let Some((wallet, wallet_index)) = wallet_and_identity_id_info {
            // If wallet information is provided, insert with wallet and wallet_index
            self.execute(
                "INSERT OR REPLACE INTO identity
             (id, data, is_local, alias, identity_type, network, wallet, wallet_index)
             VALUES (?, ?, 1, ?, ?, ?, ?, ?)",
                params![
                    id,
                    data,
                    alias,
                    identity_type,
                    network,
                    wallet,
                    wallet_index
                ],
            )?;
        } else {
            // If wallet information is not provided, insert without wallet and wallet_index
            self.execute(
                "INSERT OR REPLACE INTO identity
             (id, data, is_local, alias, identity_type, network)
             VALUES (?, ?, 1, ?, ?, ?)",
                params![id, data, alias, identity_type, network],
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
        let network = app_context.network_string();

        // Execute the update statement
        self.execute(
            "UPDATE identity
         SET data = ?, alias = ?, identity_type = ?, network = ?, is_local = 1
         WHERE id = ?",
            params![data, alias, identity_type, network, id],
        )?;

        Ok(())
    }

    pub fn insert_local_qualified_identity_in_creation(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_id: &[u8],
        identity_index: u32,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        let network = app_context.network_string();

        self.execute(
            "INSERT OR REPLACE INTO identity
         (id, data, is_local, alias, identity_type, network, is_in_creation, wallet, wallet_index)
         VALUES (?, ?, 1, ?, ?, ?, 1, ?, ?)",
            params![
                id,
                data,
                alias,
                identity_type,
                network,
                wallet_id,
                identity_index
            ],
        )?;

        Ok(())
    }

    pub fn insert_remote_identity_if_not_exists(
        &self,
        identifier: &Identifier,
        qualified_identity: Option<&QualifiedIdentity>,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let alias = qualified_identity.and_then(|qi| qi.alias.clone());
        let identity_type =
            qualified_identity.map_or("".to_string(), |qi| format!("{:?}", qi.identity_type));
        let data = qualified_identity.map(|qi| qi.to_bytes());

        let network = app_context.network_string();

        // Check if the identity already exists
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT COUNT(*) FROM identity WHERE id = ? AND network = ?")?;
        let count: i64 = stmt.query_row(params![id, network], |row| row.get(0))?;

        // If the identity doesn't exist, insert it
        if count == 0 {
            self.execute(
                "INSERT INTO identity (id, data, is_local, alias, identity_type, network)
             VALUES (?, ?, 0, ?, ?, ?)",
                params![id, data, alias, identity_type, network],
            )?;
        }

        Ok(())
    }

    pub fn get_local_qualified_identities(
        &self,
        app_context: &AppContext,
        wallets: &[Arc<RwLock<Wallet>>],
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM identity WHERE is_local = 1 AND network = ? AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let mut identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

            identity.associated_wallets = wallets.to_vec();

            Ok(identity)
        })?;

        let identities: rusqlite::Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }

    pub fn get_local_voting_identities(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM identity WHERE is_local = 1 AND network = ? AND identity_type != 'User' AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

            Ok(identity)
        })?;

        let identities: rusqlite::Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }

    pub fn get_local_user_identities(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM identity WHERE is_local = 1 AND network = ? AND identity_type = 'User' AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

            Ok(identity)
        })?;

        let identities: rusqlite::Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }

    /// Deletes a local qualified identity with the given identifier from the database.
    pub fn delete_local_qualified_identity(
        &self,
        identifier: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();

        // Perform the deletion only if the identity is marked as local
        conn.execute(
            "DELETE FROM identity WHERE id = ? AND network = ? AND is_local = 1",
            params![id, network],
        )?;

        Ok(())
    }
}
