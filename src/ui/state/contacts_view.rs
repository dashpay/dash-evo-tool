//! Contacts-tab view state for the Identities hub.
//!
//! Owns the one-shot load guard, the cached contact-request lists, the active
//! contact list, and the search query that filters it. Renders nothing — the
//! renderer lives in [`crate::ui::identity::contacts`], which reads this state
//! and paints it. Placement follows the DET module policy: non-widget UI state
//! belongs in `ui/state/`.

use super::contact_detail_view::{ContactDetailState, PendingWrite};
use crate::backend_task::dashpay::ContactData;
use crate::model::dashpay::contact_request_recipient;
use crate::model::identity_name::display_label as profile_display_label;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Document, Identifier};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Threshold for surfacing an unusually long-running request and pruning its
/// paid-action guard on the next view reset.
const REQUEST_IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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
/// The load guard debounces the backend fetch to one dispatch per tab entry.
/// View resets retain live paid-action guards and prune stale ones.
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
    /// Whether `contacts` holds a loaded list, not the empty one before the
    /// first load lands.
    contacts_known: bool,
    /// Notes the hub saved, shown over lists that may not hold them yet.
    confirmed_notes: HashMap<(Network, Identifier, Identifier), ConfirmedNote>,
    /// Established contacts flagged `display_hidden`, kept aside so the tab can
    /// offer a way back; without this list a hidden contact could only be
    /// recovered from the legacy DashPay screen.
    hidden: Vec<ContactData>,
    /// Whether the hidden-contacts section is expanded.
    show_hidden: bool,
    /// Live search query bound to the Contacts search box.
    search: String,
    /// Requests whose Accept / Decline / Cancel is already running, keyed by
    /// request ID and stamped with the dispatch time. Each of those actions is a
    /// signed, paid-for state transition, so a row keeps its buttons disabled
    /// until its result lands — a second click would buy a second transition.
    /// A matching task outcome, abandoned confirmation, or reset after
    /// [`REQUEST_IN_FLIGHT_TIMEOUT`] releases the guard.
    in_flight: HashMap<Identifier, Instant>,
    /// The identity the lists belong to, once a renderer has bound them.
    owner: Option<(Network, Identifier)>,
    /// The selected contact and its private-note editor.
    detail: ContactDetailState,
    /// The dispatch of the newest contact-list load. A reload re-arms without
    /// cancelling, so an older load can answer after a newer one; its list is
    /// stale and would undo a confirmed note.
    latest_contacts_load: Option<u64>,
}

/// A note the hub saved, shown over contact lists that may not hold it yet.
///
/// Invariant:
/// - Created only by a successful note write the hub itself dispatched, keyed
///   by the network the write ran on, the owner, and the contact. It applies
///   only to lists of that owner on that network.
/// - `after_load` is the newest contact-list load dispatched before the save
///   was confirmed. A list from that load or an earlier one, or from no load,
///   predates the save: it shows `note` whatever it holds.
/// - A list from a later load shows `note` only while it still holds
///   `lagging`, the note a read behind the save can return. A save kept on
///   this device has none: a later list already reflects it, so a different
///   note there is a published one, which wins.
/// - Retired by: a list holding `note`; a later list holding anything other
///   than `lagging`; another screen's successful write to the contact; the
///   hub's next confirmed save of it (replaced).
/// - Untouched by: the hub's failed writes, identity switches, and network
///   switches (another network's lists never match the key).
#[derive(Debug, Clone)]
struct ConfirmedNote {
    note: Option<String>,
    lagging: Option<Option<String>>,
    after_load: u64,
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

    /// Record `dispatch_id` as the newest contact-list load.
    pub fn record_contacts_load(&mut self, dispatch_id: u64) {
        self.latest_contacts_load = Some(dispatch_id);
    }

    /// Whether a contact list from load `dispatch_id` may be shown: not when a
    /// newer load was dispatched since. A list no load of this state asked for
    /// carries no dispatch and is judged as before.
    pub fn accepts_contacts_from(&self, dispatch_id: Option<u64>) -> bool {
        match (self.latest_contacts_load, dispatch_id) {
            (Some(latest), Some(dispatch_id)) => dispatch_id >= latest,
            _ => true,
        }
    }

    /// Clear cached view data while retaining guards for backend work still running.
    pub fn reset(&mut self) {
        self.load_requested = false;
        self.incoming.clear();
        self.outgoing.clear();
        self.contacts.clear();
        self.contacts_known = false;
        self.hidden.clear();
        self.show_hidden = false;
        self.search.clear();
        self.detail.reset();
        let now = Instant::now();
        self.in_flight.retain(|_, started| {
            now.saturating_duration_since(*started) < REQUEST_IN_FLIGHT_TIMEOUT
        });
    }

    /// Whether the lists have not been asked for since the load was last armed.
    pub fn load_is_due(&self) -> bool {
        !self.load_requested
    }

    /// Reset identity-scoped view data while retaining live paid-action guards.
    pub fn reset_for_identity_change(&mut self) {
        self.reset();
    }

    /// Bind the lists to the identity on screen. The selection can change
    /// without the hub hearing of it, so every renderer calls this before it
    /// reads or loads them: lists of another identity are dropped and the load
    /// re-armed, the same identity keeps its own. Paid-action guards are keyed
    /// by request, not by identity, and outlive the change — a request both
    /// identities can see stays guarded while its action runs.
    pub fn reconcile_owner(&mut self, identity: &QualifiedIdentity) {
        let owner = Some((identity.network, identity.identity.id()));
        if self.owner != owner {
            self.reset_for_identity_change();
            self.owner = owner;
        }
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
                Some(ContactRequestEntry {
                    counterpart_id: contact_request_recipient(&doc)?,
                    request_id,
                    relative_time: relative_time(&doc),
                })
            })
            .collect();
    }

    /// Store the established-contact list from `DashPayTask::LoadContacts`,
    /// split into the visible contacts and the hidden ones. Hidden contacts stay
    /// out of the active list — that is what "hidden" promises — but remain
    /// reachable through [`ContactsState::hidden_contacts`]. The selected
    /// contact's note draft follows the reloaded note.
    pub fn record_contacts(&mut self, contacts: Vec<ContactData>) {
        self.record_loaded_contacts(contacts, None);
    }

    /// [`Self::record_contacts`] for the list load `dispatch_id` returned; see
    /// [`ConfirmedNote`] for how it meets the notes the hub saved.
    pub fn record_loaded_contacts(
        &mut self,
        mut contacts: Vec<ContactData>,
        dispatch_id: Option<u64>,
    ) {
        if let Some((network, owner)) = self.owner {
            for contact in &mut contacts {
                let key = (network, owner, contact.identity_id);
                let Some(confirmed) = self.confirmed_notes.get(&key) else {
                    continue;
                };
                let from_before = dispatch_id.is_none_or(|id| id <= confirmed.after_load);
                if contact.note != confirmed.note
                    && (from_before || confirmed.lagging.as_ref() == Some(&contact.note))
                {
                    contact.note.clone_from(&confirmed.note);
                } else {
                    self.confirmed_notes.remove(&key);
                }
            }
        }
        let (hidden, visible) = contacts.into_iter().partition(|c| c.is_hidden);
        self.contacts = visible;
        self.hidden = hidden;
        self.contacts_known = true;
        if let Some(selected) = self.detail.selected()
            && let Some(contact) = self.contacts.iter().find(|c| c.identity_id == selected)
        {
            self.detail.sync(contact);
        }
    }

    /// Whether the established-contact list has been loaded for this identity.
    pub fn contacts_known(&self) -> bool {
        self.contacts_known
    }

    /// Number of established contacts, before search filtering. Drives the
    /// `Active contacts · {count}` heading.
    pub fn contacts_len(&self) -> usize {
        self.contacts.len()
    }

    /// Contacts matching the current search query, in list order. An empty or
    /// whitespace-only query matches every contact.
    pub fn matching_contacts(&self) -> impl Iterator<Item = &ContactData> {
        self.contacts
            .iter()
            .filter(|c| matches_contact_search((*c).into(), &self.search))
    }

    /// All of [`Self::matching_contacts`], for a caller that needs their number.
    pub fn filtered_contacts(&self) -> Vec<&ContactData> {
        self.matching_contacts().collect()
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
    /// authoritative reload to land. Also releases the request's in-flight
    /// guard, since its action is now resolved.
    pub fn remove_request(&mut self, request_id: &Identifier) {
        self.incoming.retain(|e| e.request_id != *request_id);
        self.outgoing.retain(|e| e.request_id != *request_id);
        self.in_flight.remove(request_id);
    }

    /// Claim the in-flight slot for a request. `true` means the caller owns the
    /// dispatch; `false` means an action for that request is already running and
    /// the caller must not dispatch a second one.
    #[must_use]
    pub fn begin_request(&mut self, request_id: Identifier) -> bool {
        if self.is_in_flight(&request_id) {
            return false;
        }
        self.in_flight.insert(request_id, Instant::now());
        true
    }

    /// Whether an action for this request is already running. Drives the row's
    /// disabled state, so the user sees why the buttons do not respond.
    pub fn is_in_flight(&self, request_id: &Identifier) -> bool {
        self.in_flight.contains_key(request_id)
    }

    /// Whether an authoritative result has taken unusually long to arrive.
    pub fn is_taking_long(&self, request_id: &Identifier) -> bool {
        self.in_flight.get(request_id).is_some_and(|started| {
            Instant::now().saturating_duration_since(*started) >= REQUEST_IN_FLIGHT_TIMEOUT
        })
    }

    /// Release one request's guard after its matching task reports a failure.
    pub fn release_request(&mut self, request_id: &Identifier) {
        self.in_flight.remove(request_id);
    }

    /// The identity the lists belong to, once bound.
    pub fn owner_id(&self) -> Option<Identifier> {
        self.owner.map(|(_, id)| id)
    }

    /// Select an active contact for the detail pane. A no-op for a contact
    /// that is not listed.
    pub fn select_contact(&mut self, contact_id: &Identifier) {
        if let Some(contact) = self.contacts.iter().find(|c| c.identity_id == *contact_id) {
            self.detail.select(contact);
        }
    }

    /// The selected contact, while it is still among the active contacts.
    pub fn selected_contact(&self) -> Option<&ContactData> {
        let selected = self.detail.selected()?;
        self.contacts.iter().find(|c| c.identity_id == selected)
    }

    /// The detail pane's state.
    pub fn detail(&self) -> &ContactDetailState {
        &self.detail
    }

    /// Mutable detail-pane state, for the note editor.
    pub fn detail_mut(&mut self) -> &mut ContactDetailState {
        &mut self.detail
    }

    /// Settle a confirmed contact details write `owner` ran on `network`;
    /// `published` when it reached Platform rather than staying on this
    /// device. Returns whether one was running. After a note save the listed contact
    /// shows the stored note right away when the lists belong to that owner on
    /// that network, without waiting for the reload; see [`ConfirmedNote`].
    pub fn finish_contact_info_write(
        &mut self,
        network: Network,
        owner: &Identifier,
        contact_id: &Identifier,
        published: bool,
    ) -> bool {
        let Some(write) = self.detail.finish_write(owner, contact_id) else {
            return false;
        };
        let PendingWrite::Note(note) = write else {
            return true;
        };
        let listed = if self.owner == Some((network, *owner)) {
            self.contacts
                .iter_mut()
                .chain(self.hidden.iter_mut())
                .find(|c| c.identity_id == *contact_id)
        } else {
            None
        };
        let lagging = listed
            .as_ref()
            .filter(|_| published)
            .map(|contact| contact.note.clone());
        self.confirmed_notes.insert(
            (network, *owner, *contact_id),
            ConfirmedNote {
                note: note.clone(),
                lagging,
                after_load: self.latest_contacts_load.unwrap_or_default(),
            },
        );
        if let Some(contact) = listed {
            contact.note = note;
            self.detail.sync(contact);
        }
        true
    }

    /// Another screen saved the contact's details after the hub did, so the
    /// hub's confirmed note is no longer the newest.
    pub fn forget_confirmed_note(
        &mut self,
        network: Network,
        owner: &Identifier,
        contact_id: &Identifier,
    ) {
        self.confirmed_notes.remove(&(network, *owner, *contact_id));
    }

    /// Release a failed or abandoned write `owner` ran on `network`, rolling
    /// back an optimistic unhide in that owner's lists on that network.
    pub fn release_contact_info_write(
        &mut self,
        network: Network,
        owner: &Identifier,
        contact_id: &Identifier,
    ) {
        if self.detail.finish_write(owner, contact_id) == Some(PendingWrite::Visibility)
            && self.owner == Some((network, *owner))
        {
            if let Some(pos) = self
                .contacts
                .iter()
                .position(|c| c.identity_id == *contact_id)
            {
                let mut contact = self.contacts.remove(pos);
                contact.is_hidden = true;
                self.hidden.push(contact);
            }
            self.invalidate();
        }
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

fn display_label(
    nickname: Option<&str>,
    display_name: Option<&str>,
    username: Option<&str>,
    id: &str,
) -> String {
    let name = nickname
        .filter(|name| !name.trim().is_empty())
        .or(display_name);
    profile_display_label(name, username, id)
}

/// Best contact label, preserving the contact's personal nickname.
pub fn contact_label(contact: &ContactData) -> String {
    display_label(
        contact.nickname.as_deref(),
        contact.display_name.as_deref(),
        contact.username.as_deref(),
        &contact.identity_id.to_string(Encoding::Base58),
    )
}

/// Pre-format a document's `created_at` as a human-relative timestamp.
fn relative_time(doc: &Document) -> Option<String> {
    let ts = doc.created_at().or_else(|| doc.updated_at())?;
    crate::ui::dashpay::format_relative_time(ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::identity::identity_pill::shorten_id;

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

    fn owned_state(owner: u8) -> ContactsState {
        let identity = dash_sdk::dpp::identity::Identity::create_basic_identity(
            id(owner),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("basic identity");
        ContactsState {
            owner: Some((Network::Testnet, identity.id())),
            ..Default::default()
        }
    }

    #[test]
    fn the_selected_contact_follows_the_active_list() {
        let mut state = ContactsState::default();
        state.record_contacts(vec![ContactData {
            note: Some("Met at the meetup".into()),
            ..contact(Some("Bao"), None, None)
        }]);
        assert!(state.selected_contact().is_none());

        state.select_contact(&id(9));
        assert!(
            state.selected_contact().is_none(),
            "an unlisted contact is not selected"
        );

        state.select_contact(&id(7));
        assert_eq!(
            state.selected_contact().and_then(|c| c.nickname.as_deref()),
            Some("Bao")
        );
        assert!(!state.detail().is_dirty(&id(7)));

        state.record_contacts(vec![ContactData {
            is_hidden: true,
            ..contact(Some("Bao"), None, None)
        }]);
        assert!(
            state.selected_contact().is_none(),
            "a contact that left the active list is no longer shown"
        );

        state.reset();
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        assert!(
            state.selected_contact().is_none(),
            "a tab entry starts unselected"
        );
    }

    #[test]
    fn a_confirmed_note_save_shows_the_stored_note_right_away() {
        let mut state = owned_state(1);
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        state.select_contact(&id(7));
        *state.detail_mut().note_mut(&id(7)).expect("draft") = "Met at the meetup".into();
        assert!(state.detail_mut().begin_save(id(1), id(7)).is_some());

        assert!(state.finish_contact_info_write(Network::Testnet, &id(1), &id(7), true));

        assert_eq!(
            state.selected_contact().and_then(|c| c.note.as_deref()),
            Some("Met at the meetup")
        );
        assert!(!state.detail().is_dirty(&id(7)));
        assert!(
            !state.finish_contact_info_write(Network::Testnet, &id(1), &id(7), true),
            "a save settles only once"
        );
    }

    #[test]
    fn confirmed_note_survives_stale_reload_and_tab_reset_until_observed() {
        for note in [Some("Confirmed"), None] {
            let mut state = owned_state(1);
            let stale = ContactData {
                note: Some("Old note".into()),
                ..contact(Some("Bao"), None, None)
            };
            state.record_contacts(vec![stale.clone()]);
            state.select_contact(&id(7));
            *state.detail_mut().note_mut(&id(7)).unwrap() = note.unwrap_or_default().into();
            assert!(state.detail_mut().begin_save(id(1), id(7)).is_some());
            assert!(state.finish_contact_info_write(Network::Testnet, &id(1), &id(7), true));
            state.reset();
            state.record_contacts(vec![stale.clone()]);
            state.select_contact(&id(7));
            assert_eq!(state.selected_contact().unwrap().note.as_deref(), note);
            assert!(!state.detail().is_dirty(&id(7)));
            state.record_contacts(vec![ContactData {
                note: note.map(str::to_string),
                ..stale.clone()
            }]);
            state.record_contacts(vec![ContactData {
                note: Some("Later remote edit".into()),
                ..stale
            }]);
            assert_eq!(
                state.selected_contact().unwrap().note.as_deref(),
                Some("Later remote edit")
            );
        }
    }

    #[test]
    fn a_note_saved_for_another_identity_leaves_these_lists_alone() {
        let mut state = owned_state(1);
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        state.select_contact(&id(7));
        *state.detail_mut().note_mut(&id(7)).expect("draft") = "Theirs".into();
        assert!(state.detail_mut().begin_save(id(2), id(7)).is_some());

        assert!(state.finish_contact_info_write(Network::Testnet, &id(2), &id(7), true));

        assert_eq!(
            state.selected_contact().and_then(|c| c.note.as_deref()),
            None
        );
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
    fn contact_info_failed_unhide_restores_only_the_owning_contacts() {
        let mut state = owned_state(1);
        state.record_contacts(vec![hidden_contact("Ghost", id(8))]);
        assert!(state.detail_mut().begin_visibility_write(id(1), id(8)));
        state.unhide_contact(&id(8));
        state.select_contact(&id(8));
        state.release_contact_info_write(Network::Testnet, &id(2), &id(8));
        assert_eq!(
            state.contacts_len(),
            1,
            "another owner's failure cannot move this row"
        );
        state.release_contact_info_write(Network::Testnet, &id(1), &id(8));
        assert_eq!(state.contacts_len(), 0);
        assert_eq!(state.hidden_contacts().len(), 1);
        assert!(state.hidden_contacts()[0].is_hidden);
        assert!(state.selected_contact().is_none());
        assert!(!state.detail().is_writing(&id(1), &id(8)));
        assert!(state.detail_mut().begin_visibility_write(id(1), id(8)));
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
    fn a_request_is_in_flight_only_once() {
        let mut state = ContactsState::default();

        assert!(
            state.begin_request(id(2)),
            "the first click owns the action"
        );
        assert!(state.is_in_flight(&id(2)));
        assert!(
            !state.begin_request(id(2)),
            "a second click must not claim an action that is already running"
        );
        assert!(
            state.begin_request(id(3)),
            "the guard is per request, not global"
        );
    }

    #[test]
    fn resolving_a_request_releases_its_guard_and_leaves_the_others() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));
        assert!(state.begin_request(id(3)));

        state.remove_request(&id(2));

        assert!(!state.is_in_flight(&id(2)));
        assert!(
            state.is_in_flight(&id(3)),
            "an unrelated request must keep its guard"
        );
    }

    #[test]
    fn releasing_one_guard_leaves_other_requests_protected() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));
        assert!(state.begin_request(id(3)));

        state.release_request(&id(2));

        assert!(!state.is_in_flight(&id(2)));
        assert!(state.is_in_flight(&id(3)));
    }

    #[test]
    fn a_guard_survives_long_enough_to_outlast_any_real_state_transition() {
        let mut state = ContactsState::default();
        state.in_flight.insert(
            id(2),
            Instant::now() - REQUEST_IN_FLIGHT_TIMEOUT + Duration::from_secs(1),
        );

        assert!(state.is_in_flight(&id(2)));
        assert!(
            !state.begin_request(id(2)),
            "a still-running paid action must not be dispatched a second time"
        );
    }

    #[test]
    fn a_guard_older_than_five_minutes_still_blocks_dispatch() {
        let mut state = ContactsState::default();
        state.in_flight.insert(
            id(2),
            Instant::now() - REQUEST_IN_FLIGHT_TIMEOUT - Duration::from_secs(1),
        );

        assert!(state.is_in_flight(&id(2)));
        assert!(state.is_taking_long(&id(2)));
        assert!(
            !state.begin_request(id(2)),
            "elapsed time must not permit a second paid action"
        );
    }

    #[test]
    fn routine_reset_preserves_the_in_flight_guards() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));

        state.reset();

        assert!(
            state.is_in_flight(&id(2)),
            "leaving the tab must not enable a second paid action while the first is still running"
        );
        assert!(
            !state.begin_request(id(2)),
            "the same request must not be dispatched again after returning to the tab"
        );
    }

    #[test]
    fn identity_change_reset_preserves_in_flight_guards_and_clears_view_state() {
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
        state.record_contacts(vec![contact(Some("Bao"), None, None)]);
        *state.search_mut() = "bao".to_string();
        assert!(state.claim_load());
        assert!(state.begin_request(id(2)));
        state.in_flight.insert(
            id(5),
            Instant::now() - REQUEST_IN_FLIGHT_TIMEOUT - Duration::from_secs(1),
        );

        state.reset_for_identity_change();

        assert!(state.incoming().is_empty());
        assert!(state.outgoing().is_empty());
        assert_eq!(state.contacts_len(), 0);
        assert!(state.search.is_empty());
        assert!(
            state.claim_load(),
            "the new identity must trigger a fresh load"
        );
        assert!(
            state.is_in_flight(&id(2)),
            "an identity switch does not cancel the paid backend action"
        );
        assert!(
            !state.begin_request(id(2)),
            "returning to the request must not dispatch it a second time"
        );
        assert!(
            !state.in_flight.contains_key(&id(5)),
            "a timed-out guard must be physically pruned from the long-lived state"
        );
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
        let base58 = id(7).to_string(Encoding::Base58);
        let fallback = contact_label(&contact(None, None, None));
        assert_eq!(fallback, shorten_id(&base58));
        assert!(fallback.contains('…'));
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
    fn a_profileless_contact_is_shortened_exactly_like_its_identity_pill() {
        // One identity must not render two different ways depending on the tab.
        // The contacts list and the identity pill share `shorten_id`, so the
        // same id shortens to the same string on both surfaces.
        let base58 = id(7).to_string(Encoding::Base58);
        assert_eq!(
            contact_label(&contact(None, None, None)),
            shorten_id(&base58),
        );
    }

    #[test]
    fn a_contacts_label_is_the_hub_wide_label_for_the_same_identity() {
        // `contact_label` must stay a pure delegation to `display_label` — the
        // whole point of the shared resolver is that no tier can drift.
        for (nickname, display, username) in [
            (Some("Bao"), Some("Alex Kim"), Some("alex.dash")),
            (None, Some("Alex Kim"), Some("alex.dash")),
            (None, None, Some("alex.dash")),
            (None, None, None),
        ] {
            let c = contact(nickname, display, username);
            assert_eq!(
                contact_label(&c),
                display_label(
                    nickname,
                    display,
                    username,
                    &c.identity_id.to_string(Encoding::Base58)
                ),
            );
        }
    }
}
