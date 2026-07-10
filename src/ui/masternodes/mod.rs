//! The Masternodes root screen domain (Expert-Mode gated).
//!
//! Node operators (the Priya persona) load masternode/evonode identities to
//! vote on DPNS name contests and manage owner/voting/payout keys. The page is
//! a sibling root screen behind the Expert-Mode nav gate (FR-1); its identities
//! are page-scoped and never leak into the everyday-user surfaces (FR-6, B1).

pub mod card;
pub mod detail_screen;
pub mod list_screen;
pub mod load_form;
pub mod testnet_fixture;

pub use list_screen::MasternodesScreen;
