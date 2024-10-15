mod derivation_path;
mod index_path;

use bincode::{Decode, Encode};

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub struct Wallet {
    pub(crate) seed: [u8; 64],
    pub alias: Option<String>,
    pub is_main: bool,
    pub password_hint: Option<String>,
}

impl Wallet {
    pub fn has_balance(&self) -> bool {
        false
    }
}
