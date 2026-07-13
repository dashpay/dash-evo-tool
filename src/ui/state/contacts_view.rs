//! Contacts-tab view state for the Identities hub.
//!
//! Owns the one-shot load guard, the cached contact-request lists, the active
//! contact list, and the search query that filters it. Renders nothing — the
//! renderer lives in [`crate::ui::identity::contacts`], which reads this state
//! and paints it. Placement follows the DET module policy: non-widget UI state
//! belongs in `ui/state/`.

use crate::backend_task::dashpay::ContactData;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Document, Identifier};

/// A single cached contact-request entry, derived from a raw
/// `DashPayContactRequests` result document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRequestEntry {
    /// Identity of the counterpart: the sender for incoming requests, the
    /// recipient for outgoing ones.
    pub counterpart_id: Identifier,
    /// The request document's own ID — routed straight into the Accept /
    /// Decline / Cancel backend tasks, so no Base58 round-trip is needed.
    pub request_id: Identifier,
    /// Human-relative timestamp (e.g. `"2 minutes ago"`), pre-formatted from
    /// the document's `created_at`. `None` when the document has no timestamp.
    pub relative_time: Option<String>,
}

/// Contacts-tab state owned by the hub screen.
///
/// The load guard debounces the backend fetch to one dispatch per tab entry;
/// the hub clears it via [`ContactsState::reset`] on refresh, tab switch, and
/// identity/network change.
#[derive(Debug, Default, Clone)]
pub struct ContactsState {
    /// `true` once the populated shell has dispatched its loads for this tab
    /// entry. Guards every subsequent frame from re-dispatching.
    load_requested: bool,
    /// Incoming requests awaiting this identity's response.
    incoming: Vec<ContactRequestEntry>,
    /// Requests this identity has sent and that are still pending.
    outgoing: Vec<ContactRequestEntry>,
    /// Established contacts the user has not hidden.
    contacts: Vec<ContactData>,
    /// Established contacts flagged `display_hidden`, kept aside so the tab can
    /// offer a way back. Declining or cancelling a request hides that person,
    /// so without this list a contact could only be recovered from the legacy
    /// DashPay screen.
    hidden: Vec<ContactData>,
    /// Whether the hidden-contacts section is expanded.
    show_hidden: bool,
    /// Live search query bound to the Contacts search box.
    search: String,
}

impl ContactsState {
    /// Claim the one-shot load slot. Returns `true` exactly once per tab entry
    /// — the caller dispatches the backend loads on `true` and skips otherwise.
    pub fn claim_load(&mut self) -> bool {
        if self.load_requested {
            return false;
        }
        self.load_requested = true;
        true
    }

    /// Clear the load guard, cached lists, and search query so the next paint
    /// re-issues the load. Called on refresh, tab switch, and identity change.
    pub fn reset(&mut self) {
        self.load_requested = false;
        self.incoming.clear();
        self.outgoing.clear();
        self.contacts.clear();
        self.hidden.clear();
        self.show_hidden = false;
        self.search.clear();
    }

    /// Re-arm the load without clearing what is already on screen. Used after a
    /// request is resolved: the authoritative lists are re-fetched while the
    /// user keeps seeing (and searching) the contacts they already had.
    pub fn invalidate(&mut self) {
        self.load_requested = false;
    }

    /// Pending requests received by this identity.
    pub fn incoming(&self) -> &[ContactRequestEntry] {
        &self.incoming
    }

    /// Pending requests sent by this identity.
    pub fn outgoing(&self) -> &[ContactRequestEntry] {
        &self.outgoing
    }

    /// Mutable handle on the search query, for binding to the search `TextEdit`.
    pub fn search_mut(&mut self) -> &mut String {
        &mut self.search
    }

    /// Populate the incoming/outgoing caches from a raw `DashPayContactRequests`
    /// backend result.
    ///
    /// Incoming sender = `doc.owner_id()`; outgoing recipient =
    /// `doc.properties()["toUserId"]`. An outgoing document whose `toUserId` is
    /// unreadable is dropped rather than shown against a default identity — a
    /// row the user cannot act on correctly is worse than no row.
    pub fn record_requests(
        &mut self,
        incoming: Vec<(Identifier, Document)>,
        outgoing: Vec<(Identifier, Document)>,
    ) {
        self.incoming = incoming
            .into_iter()
            .map(|(request_id, doc)| ContactRequestEntry {
                counterpart_id: doc.owner_id(),
                request_id,
                relative_time: relative_time(&doc),
            })
            .collect();

        self.outgoing = outgoing
            .into_iter()
            .filter_map(|(request_id, doc)| {
                let counterpart_id = doc
                    .properties()
                    .get("toUserId")
                    .and_then(|v| v.to_identifier().ok())?;
                Some(ContactRequestEntry {
                    counterpart_id,
                    request_id,
                    relative_time: relative_time(&doc),
                })
            })
            .collect();
    }

    /// Store the established-contact list from `DashPayTask::LoadContacts`,
    /// split into the visible contacts and the hidden ones. Hidden contacts stay
    /// out of the active list — that is what "hidden" promises — but remain
    /// reachable through [`ContactsState::hidden_contacts`].
    pub fn record_contacts(&mut self, contacts: Vec<ContactData>) {
        let (hidden, visible) = contacts.into_iter().partition(|c| c.is_hidden);
        self.contacts = visible;
        self.hidden = hidden;
    }

    /// Number of established contacts, before search filtering. Drives the
    /// `Active contacts · {count}` heading.
    pub fn contacts_len(&self) -> usize {
        self.contacts.len()
    }

    /// Contacts matching the current search query, in list order. An empty or
    /// whitespace-only query matches every contact.
    pub fn filtered_contacts(&self) -> Vec<&ContactData> {
        self.contacts
            .iter()
            .filter(|c| matches_contact_search((*c).into(), &self.search))
            .collect()
    }

    /// Established contacts currently flagged hidden. Never search-filtered:
    /// the section exists to make a vanished contact findable, so it always
    /// shows all of them.
    pub fn hidden_contacts(&self) -> &[ContactData] {
        &self.hidden
    }

    /// Whether the hidden-contacts section is expanded.
    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Mutable handle on the hidden-section toggle, for binding to a checkbox.
    pub fn show_hidden_mut(&mut self) -> &mut bool {
        &mut self.show_hidden
    }

    /// Move a contact out of the hidden list and back into the active one, so an
    /// unhide shows up immediately instead of waiting for the reload. A no-op
    /// when the contact is not hidden.
    pub fn unhide_contact(&mut self, contact_id: &Identifier) {
        if let Some(pos) = self
            .hidden
            .iter()
            .position(|c| c.identity_id == *contact_id)
        {
            let mut contact = self.hidden.remove(pos);
            contact.is_hidden = false;
            self.contacts.push(contact);
        }
    }

    /// Drop a resolved request (accepted, declined, or cancelled) from both
    /// lists so the row leaves the UI immediately, without waiting for the
    /// authoritative reload to land.
    pub fn remove_request(&mut self, request_id: &Identifier) {
        self.incoming.retain(|e| e.request_id != *request_id);
        self.outgoing.retain(|e| e.request_id != *request_id);
    }
}

/// The handles a contact search matches against, borrowed from whichever contact
/// type the caller holds — the Identity Hub's [`ContactData`] or the legacy
/// DashPay screen's `Contact`. One field set, so both lists find the same
/// contact for the same query.
#[derive(Debug, Clone, Copy)]
pub struct ContactSearchFields<'a> {
    pub nickname: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub username: Option<&'a str>,
    pub bio: Option<&'a str>,
    pub identity_id: Identifier,
}

impl<'a> From<&'a ContactData> for ContactSearchFields<'a> {
    fn from(contact: &'a ContactData) -> Self {
        Self {
            nickname: contact.nickname.as_deref(),
            display_name: contact.display_name.as_deref(),
            username: contact.username.as_deref(),
            bio: contact.bio.as_deref(),
            identity_id: contact.identity_id,
        }
    }
}

/// Case-insensitive substring match over every handle a user might type:
/// nickname, display name, DPNS username, bio, and the Base58 identity ID. An
/// empty or whitespace-only query matches every contact.
pub fn matches_contact_search(fields: ContactSearchFields<'_>, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [
        fields.nickname,
        fields.display_name,
        fields.username,
        fields.bio,
    ]
    .iter()
    .flatten()
    .any(|field| field.to_lowercase().contains(&needle))
        || fields
            .identity_id
            .to_string(Encoding::Base58)
            .to_lowercase()
            .contains(&needle)
}

/// Best label for a contact row: local nickname, then DashPay display name,
/// then DPNS username, then a shortened identity ID. Mirrors the hub-wide label
/// priority rule (IDH-003).
pub fn contact_label(contact: &ContactData) -> String {
    let named = [
        contact.nickname.as_deref(),
        contact.display_name.as_deref(),
        contact.username.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|s| !s.is_empty());

    match named {
        Some(name) => name.to_string(),
        None => abbreviate_id(&contact.identity_id.to_string(Encoding::Base58)),
    }
}

/// Shorten a Base58 identity ID for display: first 8 chars + "…".
pub fn abbreviate_id(id: &str) -> String {
    if id.len() <= 10 {
        id.to_string()
    } else {
        format!("{}…", &id[..8])
    }
}

/// Pre-format a document's `created_at` as a human-relative timestamp.
fn relative_time(doc: &Document) -> Option<String> {
    let ts = doc.created_at().or_else(|| doc.updated_at())?;
    crate::ui::dashpay::format_relative_time(ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    fn contact(
        nickname: Option<&str>,
        display: Option<&str>,
        username: Option<&str>,
    ) -> ContactData {
        ContactData {
            identity_id: id(7),
            nickname: nickname.map(str::to_string),
            note: None,
            is_hidden: false,
            account_reference: 0,
            username: username.map(str::to_string),
            display_name: display.map(str::to_string),
            avatar_url: None,
            bio: None,
        }
    }

    #[test]
    fn claim_load_fires_once_per_tab_entry() {
        let mut state = ContactsState::default();
        assert!(state.claim_load(), "first claim must dispatch the load");
        assert!(!state.claim_load(), "second claim must be debounced");

        state.reset();
        assert!(state.claim_load(), "reset must re-arm the load");
    }

    #[test]
    fn reset_clears_guard_lists_and_search() {
        let mut state = ContactsState::default();
        state.claim_load();
        state.incoming.push(ContactRequestEntry {
            counterpart_id: id(1),
            request_id: id(2),
            relative_time: None,
        });
        state.outgoing.push(ContactRequestEntry {
            counterpart_id: id(3),
            request_id: id(4),
            relative_time: None,
        });
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        state.search.push_str("bao");

        state.reset();

        assert!(state.claim_load(), "reset must clear the load guard");
        assert!(state.incoming().is_empty(), "reset must clear incoming");
        assert!(state.outgoing().is_empty(), "reset must clear outgoing");
        assert_eq!(state.contacts_len(), 0, "reset must clear contacts");
        assert!(state.search_mut().is_empty(), "reset must clear the search");
    }

    /// A hidden contact with a distinct identity, so it can be unhidden by ID.
    fn hidden_contact(nickname: &str, identity_id: Identifier) -> ContactData {
        ContactData {
            identity_id,
            is_hidden: true,
            ..contact(Some(nickname), None, None)
        }
    }

    #[test]
    fn record_contacts_keeps_hidden_contacts_out_of_the_active_list() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![
            contact(Some("Bao"), None, None),
            hidden_contact("Ghost", id(8)),
        ]);

        assert_eq!(
            state.contacts_len(),
            1,
            "hidden contacts must not be listed among the active ones"
        );
        assert_eq!(
            state.filtered_contacts()[0].nickname.as_deref(),
            Some("Bao")
        );
    }

    #[test]
    fn hidden_contacts_stay_reachable_so_a_contact_never_vanishes() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![
            contact(Some("Bao"), None, None),
            hidden_contact("Ghost", id(8)),
        ]);

        let hidden = state.hidden_contacts();
        assert_eq!(hidden.len(), 1, "a hidden contact must remain recoverable");
        assert_eq!(hidden[0].nickname.as_deref(), Some("Ghost"));
    }

    #[test]
    fn the_hidden_section_is_collapsed_until_the_user_opens_it() {
        let mut state = ContactsState::default();
        assert!(
            !state.show_hidden(),
            "hidden contacts stay hidden by default"
        );

        *state.show_hidden_mut() = true;
        assert!(state.show_hidden());

        state.reset();
        assert!(
            !state.show_hidden(),
            "leaving the tab must collapse the section again"
        );
    }

    #[test]
    fn unhiding_a_contact_moves_it_into_the_active_list() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![
            contact(Some("Bao"), None, None),
            hidden_contact("Ghost", id(8)),
        ]);

        state.unhide_contact(&id(8));

        assert!(
            state.hidden_contacts().is_empty(),
            "the unhidden contact must leave the hidden list"
        );
        assert_eq!(
            state.contacts_len(),
            2,
            "the unhidden contact must join the active list without waiting for a reload"
        );
        let unhidden = state
            .filtered_contacts()
            .into_iter()
            .find(|c| c.identity_id == id(8))
            .expect("the unhidden contact is now active");
        assert!(
            !unhidden.is_hidden,
            "the moved contact must no longer be flagged hidden"
        );
    }

    #[test]
    fn unhiding_an_unknown_contact_changes_nothing() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![hidden_contact("Ghost", id(8))]);

        state.unhide_contact(&id(3));

        assert_eq!(state.hidden_contacts().len(), 1);
        assert_eq!(state.contacts_len(), 0);
    }

    #[test]
    fn reset_clears_the_hidden_list() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![hidden_contact("Ghost", id(8))]);

        state.reset();

        assert!(state.hidden_contacts().is_empty());
    }

    #[test]
    fn search_matches_the_same_fields_for_every_contact_list() {
        // One matcher, one field set — the hub and the legacy screen must not
        // disagree about whether a query finds a contact.
        let mut with_bio = contact(None, None, None);
        with_bio.bio = Some("Loves kayaking".to_string());

        assert!(matches_contact_search((&with_bio).into(), "kayak"));
        assert!(matches_contact_search((&with_bio).into(), "  "));
        assert!(!matches_contact_search((&with_bio).into(), "surfing"));
    }

    #[test]
    fn empty_search_matches_every_contact() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![
            contact(Some("Bao"), None, None),
            contact(None, Some("Alex Kim"), None),
        ]);
        assert_eq!(state.filtered_contacts().len(), 2);

        state.search.push_str("   ");
        assert_eq!(
            state.filtered_contacts().len(),
            2,
            "a whitespace-only query must not filter anything out"
        );
    }

    #[test]
    fn search_filters_by_nickname_display_name_and_username() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![
            contact(Some("Bao Tran"), None, None),
            contact(None, Some("Alex Kim"), None),
            contact(None, None, Some("priya.dash")),
        ]);

        for (needle, expected) in [("bao", 1), ("ALEX", 1), ("priya", 1), ("a", 3)] {
            *state.search_mut() = needle.to_string();
            assert_eq!(
                state.filtered_contacts().len(),
                expected,
                "query '{needle}' must match {expected} contact(s)"
            );
        }
    }

    #[test]
    fn search_filters_by_base58_identity_id() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![contact(None, None, None)]);
        let base58 = id(7).to_string(Encoding::Base58);

        *state.search_mut() = base58[..6].to_string();
        assert_eq!(
            state.filtered_contacts().len(),
            1,
            "a contact with no profile must still be findable by its identity ID"
        );
    }

    #[test]
    fn search_with_no_match_returns_empty() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        *state.search_mut() = "zzzz".to_string();
        assert!(state.filtered_contacts().is_empty());
    }

    #[test]
    fn remove_request_drops_the_row_from_both_lists() {
        let mut state = ContactsState::default();
        state.incoming.push(ContactRequestEntry {
            counterpart_id: id(1),
            request_id: id(2),
            relative_time: None,
        });
        state.outgoing.push(ContactRequestEntry {
            counterpart_id: id(3),
            request_id: id(4),
            relative_time: None,
        });

        state.remove_request(&id(2));
        assert!(
            state.incoming().is_empty(),
            "accepted/declined row must leave"
        );
        assert_eq!(state.outgoing().len(), 1, "unrelated row must stay");

        state.remove_request(&id(4));
        assert!(state.outgoing().is_empty(), "cancelled row must leave");
    }

    #[test]
    fn contact_label_follows_nickname_display_username_id_priority() {
        assert_eq!(
            contact_label(&contact(Some("Bao"), Some("Alex Kim"), Some("alex.dash"))),
            "Bao"
        );
        assert_eq!(
            contact_label(&contact(None, Some("Alex Kim"), Some("alex.dash"))),
            "Alex Kim"
        );
        assert_eq!(
            contact_label(&contact(None, None, Some("alex.dash"))),
            "alex.dash"
        );

        // No profile at all — fall back to the shortened identity ID.
        let fallback = contact_label(&contact(None, None, None));
        assert_eq!(fallback, abbreviate_id(&id(7).to_string(Encoding::Base58)));
        assert!(fallback.ends_with('…'));
    }

    #[test]
    fn contact_label_ignores_blank_profile_fields() {
        assert_eq!(
            contact_label(&contact(Some("   "), Some("Alex Kim"), None)),
            "Alex Kim",
            "a whitespace-only nickname must not win the label priority"
        );
    }

    #[test]
    fn abbreviate_id_shortens_long_ids_only() {
        assert_eq!(abbreviate_id("AbCdEfGhIjKlMnOpQrStUv"), "AbCdEfGh…");
        assert_eq!(abbreviate_id("AbCdEfGh"), "AbCdEfGh");
        assert_eq!(abbreviate_id(""), "");
    }
}
