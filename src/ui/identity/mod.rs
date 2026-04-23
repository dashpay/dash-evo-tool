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
//! The module is compiled unconditionally. The `identity-hub` Cargo feature
//! only controls whether the left-nav entry is rendered — so toggling the
//! feature cannot leave the screen enum with unreachable variants.

pub mod activity;
pub mod contacts;
pub mod home;
pub mod hub_screen;
pub mod landing;
pub mod onboarding;
pub mod picker;
pub mod settings;
pub mod tabs;

pub use hub_screen::IdentityHubScreen;
pub use tabs::IdentityHubTab;
