#[derive(Debug, Clone)]
pub struct PasswordInfo {
    pub password_checker: Vec<u8>,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
}
