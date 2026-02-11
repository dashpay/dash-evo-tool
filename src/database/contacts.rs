use dash_sdk::platform::Identifier;
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
pub struct ContactPrivateInfo {
    pub owner_identity_id: Vec<u8>,
    pub contact_identity_id: Vec<u8>,
    pub nickname: String,
    pub notes: String,
    pub is_hidden: bool,
}

impl crate::database::Database {
    pub fn init_contacts_tables(&self, conn: &Connection) -> rusqlite::Result<()> {
        let sql = "
            CREATE TABLE IF NOT EXISTS contact_private_info (
                owner_identity_id BLOB NOT NULL,
                contact_identity_id BLOB NOT NULL,
                nickname TEXT,
                notes TEXT,
                is_hidden INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (unixepoch()),
                updated_at INTEGER DEFAULT (unixepoch()),
                PRIMARY KEY (owner_identity_id, contact_identity_id)
            );
        ";
        conn.execute(sql, [])?;
        Ok(())
    }

    pub fn save_contact_private_info(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        nickname: &str,
        notes: &str,
        is_hidden: bool,
    ) -> rusqlite::Result<()> {
        let sql = "
            INSERT OR REPLACE INTO contact_private_info
            (owner_identity_id, contact_identity_id, nickname, notes, is_hidden, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())
        ";

        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
                nickname,
                notes,
                is_hidden as i32,
            ],
        )?;
        Ok(())
    }

    pub fn load_contact_private_info(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<(String, String, bool)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT nickname, notes, is_hidden FROM contact_private_info
             WHERE owner_identity_id = ?1 AND contact_identity_id = ?2",
        )?;

        let result = stmt.query_row(
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, i32>(2).unwrap_or(0) != 0,
                ))
            },
        );

        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((String::new(), String::new(), false)),
            Err(e) => Err(e),
        }
    }

    pub fn load_all_contact_private_info(
        &self,
        owner_identity_id: &Identifier,
    ) -> rusqlite::Result<Vec<ContactPrivateInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT owner_identity_id, contact_identity_id, nickname, notes, is_hidden
             FROM contact_private_info
             WHERE owner_identity_id = ?1",
        )?;

        let infos = stmt
            .query_map(params![owner_identity_id.to_buffer().to_vec()], |row| {
                Ok(ContactPrivateInfo {
                    owner_identity_id: row.get(0)?,
                    contact_identity_id: row.get(1)?,
                    nickname: row.get(2)?,
                    notes: row.get(3)?,
                    is_hidden: row.get::<_, i32>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(infos)
    }

    pub fn delete_contact_private_info(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
    ) -> rusqlite::Result<()> {
        let sql = "DELETE FROM contact_private_info WHERE owner_identity_id = ?1 AND contact_identity_id = ?2";
        self.execute(
            sql,
            params![
                owner_identity_id.to_buffer().to_vec(),
                contact_identity_id.to_buffer().to_vec(),
            ],
        )?;
        Ok(())
    }

    /// Toggle or set the hidden status for a contact
    /// Creates a new entry if one doesn't exist
    pub fn set_contact_hidden(
        &self,
        owner_identity_id: &Identifier,
        contact_identity_id: &Identifier,
        is_hidden: bool,
    ) -> rusqlite::Result<()> {
        // First try to load existing info to preserve nickname and notes
        let (nickname, notes, _) =
            self.load_contact_private_info(owner_identity_id, contact_identity_id)?;

        // Save with updated hidden status
        self.save_contact_private_info(
            owner_identity_id,
            contact_identity_id,
            &nickname,
            &notes,
            is_hidden,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_helpers::create_test_database;

    fn create_test_identifier() -> Identifier {
        Identifier::random()
    }

    #[test]
    fn test_save_and_retrieve_contact_private_info() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Save contact info
        db.save_contact_private_info(&owner_id, &contact_id, "Alice", "My best friend", false)
            .expect("Failed to save contact info");

        // Retrieve it
        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "Alice");
        assert_eq!(notes, "My best friend");
        assert!(!is_hidden);
    }

    #[test]
    fn test_contact_private_info_not_found_returns_defaults() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Try to load non-existent contact info
        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "");
        assert_eq!(notes, "");
        assert!(!is_hidden);
    }

    #[test]
    fn test_update_contact_private_info() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Save initial info
        db.save_contact_private_info(&owner_id, &contact_id, "Alice", "Note 1", false)
            .expect("Failed to save contact info");

        // Update it
        db.save_contact_private_info(&owner_id, &contact_id, "Bob", "Note 2", true)
            .expect("Failed to update contact info");

        // Retrieve updated info
        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "Bob");
        assert_eq!(notes, "Note 2");
        assert!(is_hidden);
    }

    #[test]
    fn test_delete_contact_private_info() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Save contact info
        db.save_contact_private_info(&owner_id, &contact_id, "Alice", "Notes", false)
            .expect("Failed to save contact info");

        // Delete it
        db.delete_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to delete contact info");

        // Should return defaults now
        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "");
        assert_eq!(notes, "");
        assert!(!is_hidden);
    }

    #[test]
    fn test_load_all_contact_private_info() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();

        // Add multiple contacts
        for i in 0..5 {
            let contact_id = create_test_identifier();
            db.save_contact_private_info(
                &owner_id,
                &contact_id,
                &format!("Contact {}", i),
                &format!("Notes for contact {}", i),
                i % 2 == 0, // Every other contact is hidden
            )
            .expect("Failed to save contact info");
        }

        // Load all contacts for this owner
        let contacts = db
            .load_all_contact_private_info(&owner_id)
            .expect("Failed to load all contacts");

        assert_eq!(contacts.len(), 5);

        // Verify hidden status pattern
        let hidden_count = contacts.iter().filter(|c| c.is_hidden).count();
        assert_eq!(hidden_count, 3); // 0, 2, 4 are hidden
    }

    #[test]
    fn test_set_contact_hidden_new_contact() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Set hidden on a new contact (should create entry)
        db.set_contact_hidden(&owner_id, &contact_id, true)
            .expect("Failed to set contact hidden");

        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "");
        assert_eq!(notes, "");
        assert!(is_hidden);
    }

    #[test]
    fn test_set_contact_hidden_preserves_existing_data() {
        let db = create_test_database().expect("Failed to create test database");
        let owner_id = create_test_identifier();
        let contact_id = create_test_identifier();

        // Save contact info with nickname and notes
        db.save_contact_private_info(&owner_id, &contact_id, "Alice", "Important notes", false)
            .expect("Failed to save contact info");

        // Change hidden status
        db.set_contact_hidden(&owner_id, &contact_id, true)
            .expect("Failed to set contact hidden");

        // Verify nickname and notes are preserved
        let (nickname, notes, is_hidden) = db
            .load_contact_private_info(&owner_id, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname, "Alice");
        assert_eq!(notes, "Important notes");
        assert!(is_hidden);
    }

    #[test]
    fn test_contacts_isolation_between_owners() {
        let db = create_test_database().expect("Failed to create test database");
        let owner1 = create_test_identifier();
        let owner2 = create_test_identifier();
        let contact_id = create_test_identifier();

        // Both owners have the same contact but with different info
        db.save_contact_private_info(
            &owner1,
            &contact_id,
            "Alice (Owner1)",
            "Notes from 1",
            false,
        )
        .expect("Failed to save contact info");
        db.save_contact_private_info(&owner2, &contact_id, "Alice (Owner2)", "Notes from 2", true)
            .expect("Failed to save contact info");

        // Verify isolation
        let (nickname1, notes1, hidden1) = db
            .load_contact_private_info(&owner1, &contact_id)
            .expect("Failed to load contact info");
        let (nickname2, notes2, hidden2) = db
            .load_contact_private_info(&owner2, &contact_id)
            .expect("Failed to load contact info");

        assert_eq!(nickname1, "Alice (Owner1)");
        assert_eq!(notes1, "Notes from 1");
        assert!(!hidden1);

        assert_eq!(nickname2, "Alice (Owner2)");
        assert_eq!(notes2, "Notes from 2");
        assert!(hidden2);
    }
}
