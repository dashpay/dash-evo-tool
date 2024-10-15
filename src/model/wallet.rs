use bincode::{Decode, Encode};

#[derive(Debug, Encode, Decode)]
pub struct Wallet {
    pub(crate) seed: [u8; 64],
    pub alias: Option<String>,
    pub is_main: bool,
    pub password_hint: Option<String>,
}
