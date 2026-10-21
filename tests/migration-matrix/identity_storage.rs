//! Identity record encoding shared by the migration fixture checks.

use dash_evo_tool::wallet_backend::KV_SCHEMA_VERSION;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Key used by `context::identity_db` in the per-network identity table.
pub const IDENTITY_KEY: &str = "det:identity:v1";

/// Mirrors `context::identity_db::StoredQualifiedIdentity`, including its bincode field order.
#[derive(Deserialize, Serialize)]
pub struct StoredIdentity {
    pub qi_bytes: Vec<u8>,
    pub status: u8,
    pub identity_type: String,
    pub wallet_hash: Option<[u8; 32]>,
    pub wallet_index: Option<u32>,
}

impl StoredIdentity {
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let Some((&KV_SCHEMA_VERSION, body)) = bytes.split_first() else {
            return Err("unexpected identity k/v schema version".into());
        };
        let (stored, consumed) =
            bincode::serde::decode_from_slice(body, bincode::config::standard())
                .map_err(|e| e.to_string())?;
        if consumed != body.len() {
            return Err("unexpected trailing data in identity record".into());
        }
        Ok(stored)
    }

    /// Encode the record with the same schema header used by the storage adapter.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut bytes = vec![KV_SCHEMA_VERSION];
        bytes.extend(
            bincode::serde::encode_to_vec(self, bincode::config::standard())
                .map_err(|e| e.to_string())?,
        );
        Ok(bytes)
    }
}

/// Read versioned identity records with their storage IDs, ordered by ID.
pub fn read_identities(conn: &Connection) -> Result<Vec<(Vec<u8>, StoredIdentity)>, String> {
    let mut statement = conn
        .prepare("SELECT identity_id, value FROM meta_identity WHERE key = ?1 ORDER BY identity_id")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([IDENTITY_KEY], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| {
        let (id, bytes) = row.map_err(|e| e.to_string())?;
        Ok((id, StoredIdentity::decode(&bytes)?))
    })
    .collect()
}
