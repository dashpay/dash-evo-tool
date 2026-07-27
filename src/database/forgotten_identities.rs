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

    /// List every identity deliberately unloaded on one network.
    pub(crate) fn list_forgotten_identities(
        &self,
        network: Network,
    ) -> rusqlite::Result<Vec<Identifier>> {
        let conn = self.locked_conn();
        let mut statement = conn.prepare(
            "SELECT identity_id FROM forgotten_identities
             WHERE network = ?1",
        )?;
        let rows = statement.query_map(params![network.to_string()], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            let id: [u8; 32] = bytes.as_slice().try_into().map_err(|source| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    Box::new(source),
                )
            })?;
            Ok(Identifier::from(id))
        })?;
        rows.collect()
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

    #[test]
    fn forgotten_identity_listing_is_network_scoped() {
        let db = create_test_database().expect("create database");
        let testnet_ids = [Identifier::from([0x41; 32]), Identifier::from([0x42; 32])];
        let mainnet_id = Identifier::from([0x43; 32]);

        for identity_id in &testnet_ids {
            db.record_forgotten_identity(Network::Testnet, identity_id)
                .expect("record testnet marker");
        }
        db.record_forgotten_identity(Network::Mainnet, &mainnet_id)
            .expect("record mainnet marker");

        let mut listed = db
            .list_forgotten_identities(Network::Testnet)
            .expect("list testnet markers");
        listed.sort_unstable();
        let mut expected = testnet_ids.to_vec();
        expected.sort_unstable();

        assert_eq!(listed, expected);
        assert!(
            !listed.contains(&mainnet_id),
            "identities from another network must not be returned"
        );
    }
}
