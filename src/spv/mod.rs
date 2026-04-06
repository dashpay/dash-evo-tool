mod error;
pub(crate) mod manager;
#[cfg(test)]
mod tests;

pub use error::{SpvError, SpvResult};
pub use manager::{CoreBackendMode, SpvDerivedAddress, SpvManager, SpvStatus, SpvStatusSnapshot};
