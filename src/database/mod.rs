mod contested_names;
mod identities;
mod initialization;

use dpp::identity::accessors::IdentityGettersV0;
use dpp::serialization::PlatformSerializable;
use rusqlite::Connection;
use std::path::Path;
#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }
}
