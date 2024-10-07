use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use dpp::identifier::Identifier;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::Identity;
use dpp::serialization::PlatformSerializable;
use rusqlite::{params, Connection, Result};
use std::path::Path;

#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }
    pub fn initialize(&self) -> Result<()> {
        // Create the identities table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS identity (
                id BLOB PRIMARY KEY,
                data BLOB,
                is_local INTEGER NOT NULL,
                network TEXT NOT NULL,
                alias TEXT,
                info TEXT,
                identity_type TEXT
            )",
            [],
        )?;

        // Create the contested names table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS contested_name (
                normalized_contested_name TEXT PRIMARY KEY,
                locked_votes INTEGER,
                abstain_votes INTEGER,
                awarded_to BLOB,
                ending_time INTEGER
            )",
            [],
        )?;

        // Create the contestants table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS contestant (
                contest_id TEXT,
                identity_id BLOB,
                name TEXT,
                votes INTEGER,
                PRIMARY KEY (contest_id, identity_id),
                FOREIGN KEY (contest_id) REFERENCES contested_names(contest_id),
                FOREIGN KEY (identity_id) REFERENCES identities(id)
            )",
            [],
        )?;
        Ok(())
    }

    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        app_context: &AppContext,
    ) -> Result<()> {
        let id = qualified_identity.identity.id().to_vec();
        let data = qualified_identity.to_bytes();
        let alias = qualified_identity.alias.clone();
        let identity_type = format!("{:?}", qualified_identity.identity_type);

        let network = app_context.network_string();

        self.conn.execute(
            "INSERT OR REPLACE INTO identity (id, data, is_local, network, alias, identity_type)
         VALUES (?, ?, 1, ?, ?, ?)",
            params![id, data, network, alias, identity_type],
        )?;
        Ok(())
    }

    pub fn insert_remote_identity(
        &self,
        identifier: &Identifier,
        qualified_identity: Option<&QualifiedIdentity>,
        app_context: &AppContext,
    ) -> Result<()> {
        let id = identifier.to_vec();
        let alias = qualified_identity.and_then(|qi| qi.alias.clone());
        let identity_type =
            qualified_identity.map_or("".to_string(), |qi| format!("{:?}", qi.identity_type));
        let data = qualified_identity.map(|qi| qi.to_bytes());

        let network = app_context.network_string();

        self.conn.execute(
            "INSERT OR REPLACE INTO identity (id, data, is_local, network, alias, identity_type)
         VALUES (?, ?, 0, ?, ?, ?)",
            params![id, data, network, alias, identity_type],
        )?;
        Ok(())
    }

    pub fn get_local_qualified_identities(
        &self,
        app_context: &AppContext,
    ) -> Result<Vec<QualifiedIdentity>> {
        let network = app_context.network_string();

        let mut stmt = self.conn.prepare(
            "SELECT id, data, alias, identity_type FROM identity WHERE is_local = 1 AND network = ? AND data IS NOT NULL",
        )?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(1)?;
            let identity: QualifiedIdentity = QualifiedIdentity::from_bytes(&data);

            Ok(identity)
        })?;

        let identities: Result<Vec<QualifiedIdentity>> = identity_iter.collect();
        identities
    }
}
