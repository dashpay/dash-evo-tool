//! Unified Identities hub UI section.
//!
//! This module implements the four-tab hub described in
//! `docs/ai-design/2026-04-22-identity-dashpay-redesign/`:
//! Home · Contacts · Activity · Settings, preceded by an onboarding empty state
//! and an identity picker grid for multi-identity contexts.
//!
//! The hub coexists with the legacy `src/ui/identities/` and `src/ui/dashpay/`
//! screens during the transition; both old nav entries remain visible.
//!
//! See the planning artifacts at
//! `docs/ai-design/2026-04-23-identity-hub-impl/` (requirements, UX plan,
//! test-case spec, dev plan).
//!
//! The module is compiled unconditionally so the screen and enum variants are
//! always present (avoiding unreachable-variant errors in match dispatch).
//! The `identity-hub` Cargo feature controls two integration sites:
//!
//! 1. the left-nav `Identity Hub` entry in `src/ui/components/left_panel.rs`
//!    (rendered only when the feature is on), and
//! 2. the `main_screens` map in `src/app.rs::AppState::new` — the
//!    `RootScreenIdentityHub` entry is inserted only when the feature is on.
//!
//! When the feature is off, the screen type still compiles but is not
//! registered in `AppState`, so `AppState::active_root_screen_mut()` must
//! guard against a persisted `RootScreenIdentityHub` selection by falling
//! back to a registered screen (handled by the resolver in `AppState::new`).

pub mod activity;
pub mod activity_row;
pub mod avatar;
pub mod breadcrumb_switcher;
pub mod contact_row;
pub mod contacts;
pub mod home;
pub mod hub_screen;
pub mod identity_hero_card;
pub mod identity_hub_tab_bar;
pub mod identity_picker_add_card;
pub mod identity_picker_card;
pub mod identity_pill;
pub mod landing;
pub mod onboarding;
pub mod onboarding_checklist;
pub mod picker;
pub mod profile_cache;
pub mod request_card;
pub mod settings;
pub mod social_profile_gate_card;
pub mod tabs;

pub use hub_screen::IdentityHubScreen;
pub use tabs::IdentityHubTab;
