use dash_sdk::platform::Identifier;
use rusqlite::params;

impl crate::database::Database {
    pub fn init_contacts_tables(&self) -> rusqlite::Result<()> {
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
        self.execute(sql, [])?;
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
}
