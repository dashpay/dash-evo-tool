pub mod address_input;
pub mod amount_input;
pub mod breadcrumb_pill;
pub mod component_trait;
pub mod confirmation_dialog;
pub mod contract_chooser_panel;
pub mod dashpay_subscreen_chooser_panel;
pub mod dpns_subscreen_chooser_panel;
pub mod entropy_grid;
pub mod icons;
pub mod identity_selector;
pub mod info_popup;
pub mod left_panel;
pub mod message_banner;
pub mod passphrase_modal;
pub mod password_input;
pub mod progress_overlay;
pub mod secret_prompt_host;
pub mod selection_dialog;
pub mod styled;
pub mod tokens_subscreen_chooser_panel;
pub mod tools_subscreen_chooser_panel;
pub mod top_panel;
pub mod wallet_unlock_popup;

// Re-export the main traits for easy access
pub use component_trait::{Component, ComponentResponse};
pub use message_banner::{
    BannerHandle, BannerStatus, MessageBanner, MessageBannerResponse, OptionBannerExt,
    OptionBannerShowExt, ResultBannerExt,
};
pub use progress_overlay::{
    OptionOverlayExt, OverlayConfig, OverlayHandle, ProgressOverlay, ProgressOverlayResponse,
};
pub use secret_prompt_host::{ActivePrompt, EguiSecretPromptHost, QueuedPrompt};
