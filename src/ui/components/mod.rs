pub mod amount_input;
pub mod component_trait;
pub mod confirmation_dialog;
pub mod contract_chooser_panel;
pub mod dpns_subscreen_chooser_panel;
pub mod entropy_grid;
pub mod identity_selector;
pub mod key_selector;
pub mod left_panel;
pub mod left_wallet_panel;
pub mod styled;
pub mod tokens_subscreen_chooser_panel;
pub mod tools_subscreen_chooser_panel;
pub mod top_panel;
pub mod wallet_unlock;

// Re-export the main traits for easy access
pub use component_trait::{Component, ComponentResponse};
