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
//! TODO: `identity/` (this new hub) vs `identities/` (the legacy tree) is a
//! one-letter module-name collision that will keep confusing imports, greps,
//! and file-picker jumps as long as both exist. Either rename this tree to
//! something unambiguous (e.g. `identity_hub/`) or open a tracked issue with
//! a target release for folding `identities/` into the hub, so the near-miss
//! naming is bounded in time rather than permanent.
//!
//! See the planning artifacts at
//! `docs/ai-design/2026-04-23-identity-hub-impl/` (requirements, UX plan,
//! test-case spec, dev plan).
//!
//! Integration sites: the left-nav `Identity Hub` entry in
//! `src/ui/components/left_panel.rs`, and the `RootScreenIdentityHub` entry in
//! the `main_screens` map in `src/app.rs::AppState::new`.

pub mod activity;
pub mod avatar;
pub mod breadcrumb_switcher;
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
