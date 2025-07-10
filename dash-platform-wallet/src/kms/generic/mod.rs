pub mod key_handle;
pub mod locked;
pub mod unlocked;

pub use locked::GenericKms;

pub const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM
