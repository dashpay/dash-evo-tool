use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use rusqlite::params;

impl Database {
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        let network = app_context.network_string();

        self.execute(
            "INSERT OR REPLACE INTO identity (id, data, is_local, alias, identity_type, network)
         VALUES (?, ?, 1, ?, ?, ?)",
            params![id, data, alias, identity_type, network],
        )?;
        Ok(())
    }

    pub fn get_local_qualified_identities(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM identity WHERE is_local = 1 AND network = ? AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

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
}
