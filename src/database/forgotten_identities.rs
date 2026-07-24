use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;
use rusqlite::{Connection, params};

use super::Database;

impl Database {
    /// Create the durable per-network identity-unload marker table.
    pub(crate) fn initialize_forgotten_identities_table(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS forgotten_identities (
                network TEXT NOT NULL,
                identity_id BLOB NOT NULL CHECK (length(identity_id) = 32),
                PRIMARY KEY (network, identity_id)
            )",
            [],
        )?;
        Ok(())
    }

    /// Record that automatic discovery must not restore an unloaded identity.
    pub(crate) fn record_forgotten_identity(
        &self,
        network: Network,
        identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        self.execute(
            "INSERT OR IGNORE INTO forgotten_identities (network, identity_id)
             VALUES (?1, ?2)",
            params![network.to_string(), identity_id.to_buffer()],
        )?;
        Ok(())
    }

    /// Allow discovery after the user explicitly restores an identity.
    pub(crate) fn clear_forgotten_identity(
        &self,
        network: Network,
        identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        self.execute(
            "DELETE FROM forgotten_identities
             WHERE network = ?1 AND identity_id = ?2",
            params![network.to_string(), identity_id.to_buffer()],
        )?;
        Ok(())
    }

    /// Whether an identity is deliberately unloaded on one network.
    pub(crate) fn is_identity_forgotten(
        &self,
        network: Network,
        identity_id: &Identifier,
    ) -> rusqlite::Result<bool> {
        let conn = self.locked_conn();
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM forgotten_identities
                WHERE network = ?1 AND identity_id = ?2
             )",
            params![network.to_string(), identity_id.to_buffer()],
            |row| row.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    #[test]
    fn forgotten_identity_markers_are_durable_per_network() {
        let db = create_test_database().expect("create database");
        let identity_id = Identifier::from([0x31; 32]);

        db.record_forgotten_identity(Network::Testnet, &identity_id)
            .expect("record marker");

        assert!(
            db.is_identity_forgotten(Network::Testnet, &identity_id)
                .expect("read testnet marker")
        );
        assert!(
            !db.is_identity_forgotten(Network::Mainnet, &identity_id)
                .expect("read mainnet marker")
        );

        db.clear_forgotten_identity(Network::Testnet, &identity_id)
            .expect("clear marker");
        assert!(
            !db.is_identity_forgotten(Network::Testnet, &identity_id)
                .expect("read cleared marker")
        );
    }
}
