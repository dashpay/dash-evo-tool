use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::platform::Identifier;
use dpp::identity::accessors::IdentityGettersV0;
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

        self.conn.execute(
            "INSERT OR REPLACE INTO identity (id, data, is_local, alias, identity_type, network)
         VALUES (?, ?, 1, ?, ?, ?)",
            params![id, data, alias, identity_type, network],
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
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM identity WHERE id = ? AND network = ?")?;
        let count: i64 = stmt.query_row(params![id, network], |row| row.get(0))?;

        // If the identity doesn't exist, insert it
        if count == 0 {
            self.conn.execute(
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
    ) -> rusqlite::Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let mut stmt = self.conn.prepare(
            "SELECT id, data, alias, identity_type FROM identity WHERE is_local = 1 AND network = ? AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(1)?;
            let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

            Ok(identity)
        })?;

        let identities: rusqlite::Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }
}
