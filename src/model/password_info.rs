#[derive(Debug, Clone)]
pub struct PasswordInfo {
    #[allow(dead_code)] // Used for password verification
    pub password_checker: Vec<u8>,
    #[allow(dead_code)] // Used for password hashing
    pub salt: Vec<u8>,
    #[allow(dead_code)] // Used for encryption
    pub nonce: Vec<u8>,
}
