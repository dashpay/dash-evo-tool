//! Detail-pane state for the Identities hub's Contacts tab: which contact is
//! selected, the private note being edited for it, and the contact details
//! writes still running. Renders nothing — [`crate::ui::identity::contacts`]
//! paints it.

use crate::backend_task::dashpay::ContactData;
use dash_sdk::platform::Identifier;
use std::collections::HashMap;

/// A contact details write still running. Each rewrites the contact's whole
/// `contactInfo` document, so a contact has at most one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingWrite {
    /// A private-note save, with the note that was sent.
    Note(Option<String>),
    /// An unhide.
    Visibility,
}

/// The note being edited for one contact, and the stored note it started from.
#[derive(Debug, Clone)]
struct NoteDraft {
    contact_id: Identifier,
    baseline: Option<String>,
    text: String,
}

/// Selection and private-note editing for the contact detail pane.
#[derive(Debug, Default, Clone)]
pub struct ContactDetailState {
    selected: Option<Identifier>,
    draft: Option<NoteDraft>,
    /// Contact details writes still running, keyed by owner identity and
    /// contact. Each is paid for, so the next write to that contact waits until
    /// its result lands.
    writes: HashMap<(Identifier, Identifier), PendingWrite>,
}

/// The note as it is stored: surrounding whitespace dropped, and an empty note
/// stored as no note.
pub fn normalized_note(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl NoteDraft {
    fn new(contact: &ContactData) -> Self {
        let baseline = contact.note.as_deref().and_then(normalized_note);
        Self {
            contact_id: contact.identity_id,
            text: baseline.clone().unwrap_or_default(),
            baseline,
        }
    }

    fn is_dirty(&self) -> bool {
        normalized_note(&self.text) != self.baseline
    }
}

impl ContactDetailState {
    /// Select `contact` and start its note draft from the stored note.
    /// Selecting the contact already selected keeps its draft.
    pub fn select(&mut self, contact: &ContactData) {
        if self.draft_for(&contact.identity_id).is_some() {
            return;
        }
        self.selected = Some(contact.identity_id);
        self.draft = Some(NoteDraft::new(contact));
    }

    /// The selected contact, if any.
    pub fn selected(&self) -> Option<Identifier> {
        self.selected
    }

    /// Drop the selection and draft, retaining write guards until their tasks finish.
    pub fn reset(&mut self) {
        self.selected = None;
        self.draft = None;
    }

    /// Follow a fresh copy of the selected contact. The draft restarts from the
    /// stored note when the user has not changed it, and an edit the stored
    /// note now matches is no longer unsaved; any other edit is kept.
    pub fn sync(&mut self, contact: &ContactData) {
        if self.selected != Some(contact.identity_id) {
            return;
        }
        let stored = contact.note.as_deref().and_then(normalized_note);
        match &mut self.draft {
            Some(draft) if draft.contact_id == contact.identity_id => {
                if !draft.is_dirty() {
                    draft.text = stored.clone().unwrap_or_default();
                    draft.baseline = stored;
                } else if normalized_note(&draft.text) == stored {
                    draft.baseline = stored;
                }
            }
            _ => self.draft = Some(NoteDraft::new(contact)),
        }
    }

    fn draft_for(&self, contact_id: &Identifier) -> Option<&NoteDraft> {
        self.draft
            .as_ref()
            .filter(|draft| draft.contact_id == *contact_id)
    }

    /// The draft text for `contact_id`, for binding to the note editor.
    pub fn note_mut(&mut self, contact_id: &Identifier) -> Option<&mut String> {
        self.draft
            .as_mut()
            .filter(|draft| draft.contact_id == *contact_id)
            .map(|draft| &mut draft.text)
    }

    /// The note the draft would store for `contact_id`.
    pub fn note_to_save(&self, contact_id: &Identifier) -> Option<String> {
        self.draft_for(contact_id)
            .and_then(|draft| normalized_note(&draft.text))
    }

    /// Whether the draft for `contact_id` differs from the stored note.
    pub fn is_dirty(&self, contact_id: &Identifier) -> bool {
        self.draft_for(contact_id).is_some_and(NoteDraft::is_dirty)
    }

    /// Claim the write slot for `owner`'s note on `contact_id`. Returns the note
    /// to send, or `None` when there is no draft for that contact or a write to
    /// it is already running.
    pub fn begin_save(
        &mut self,
        owner: Identifier,
        contact_id: Identifier,
    ) -> Option<Option<String>> {
        let draft = self.draft_for(&contact_id)?;
        let note = normalized_note(&draft.text);
        self.begin_write(owner, contact_id, PendingWrite::Note(note.clone()))
            .then_some(note)
    }

    /// Claim the write slot for unhiding `contact_id`. `false` when a write to
    /// that contact is already running.
    #[must_use]
    pub fn begin_visibility_write(&mut self, owner: Identifier, contact_id: Identifier) -> bool {
        self.begin_write(owner, contact_id, PendingWrite::Visibility)
    }

    fn begin_write(
        &mut self,
        owner: Identifier,
        contact_id: Identifier,
        write: PendingWrite,
    ) -> bool {
        let key = (owner, contact_id);
        if self.writes.contains_key(&key) {
            return false;
        }
        self.writes.insert(key, write);
        true
    }

    /// Whether a contact details write for `owner` and `contact_id` is running.
    pub fn is_writing(&self, owner: &Identifier, contact_id: &Identifier) -> bool {
        self.writes.contains_key(&(*owner, *contact_id))
    }

    /// Release a confirmed write and return what it was.
    pub fn finish_write(
        &mut self,
        owner: &Identifier,
        contact_id: &Identifier,
    ) -> Option<PendingWrite> {
        self.writes.remove(&(*owner, *contact_id))
    }

    /// Release a write that failed or was abandoned; the draft is kept.
    pub fn release_write(&mut self, owner: &Identifier, contact_id: &Identifier) {
        self.writes.remove(&(*owner, *contact_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    fn contact(byte: u8, note: Option<&str>) -> ContactData {
        ContactData {
            identity_id: id(byte),
            nickname: None,
            note: note.map(str::to_string),
            is_hidden: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }
    }

    #[test]
    fn whitespace_only_notes_are_stored_as_no_note() {
        assert_eq!(normalized_note(""), None);
        assert_eq!(normalized_note("  \n"), None);
        assert_eq!(normalized_note("  Bao\n"), Some("Bao".into()));
    }

    #[test]
    fn selecting_a_contact_starts_from_its_stored_note() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, Some("Met at the meetup")));

        assert_eq!(state.selected(), Some(id(5)));
        assert_eq!(
            state.note_mut(&id(5)).map(|note| note.as_str()),
            Some("Met at the meetup")
        );
        assert!(
            !state.is_dirty(&id(5)),
            "an untouched note has nothing to save"
        );
        assert_eq!(
            state.note_mut(&id(6)),
            None,
            "the draft is only for the selection"
        );
    }

    #[test]
    fn editing_the_note_makes_it_dirty_until_it_matches_again() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, Some("Bao")));
        *state.note_mut(&id(5)).expect("draft") = "Bao from work".into();
        assert!(state.is_dirty(&id(5)));
        assert_eq!(state.note_to_save(&id(5)), Some("Bao from work".into()));

        *state.note_mut(&id(5)).expect("draft") = " Bao ".into();
        assert!(
            !state.is_dirty(&id(5)),
            "surrounding whitespace does not change what is stored"
        );
    }

    #[test]
    fn a_reloaded_note_replaces_an_untouched_draft_but_not_an_edit() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        state.sync(&contact(5, Some("Saved elsewhere")));
        assert_eq!(
            state.note_mut(&id(5)).map(|note| note.clone()),
            Some("Saved elsewhere".into())
        );

        *state.note_mut(&id(5)).expect("draft") = "My edit".into();
        state.sync(&contact(5, Some("Saved again")));
        assert_eq!(
            state.note_mut(&id(5)).map(|note| note.clone()),
            Some("My edit".into()),
            "an edit in progress is never overwritten"
        );
    }

    #[test]
    fn only_the_selected_contact_moves_the_draft() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "My edit".into();

        state.select(&contact(5, None));
        state.sync(&contact(6, Some("Someone else")));
        assert_eq!(
            state.note_mut(&id(5)).map(|note| note.clone()),
            Some("My edit".into()),
            "re-selecting the contact or another contact's reload keeps the edit"
        );

        state.select(&contact(6, Some("Theirs")));
        assert_eq!(state.selected(), Some(id(6)));
        assert_eq!(
            state.note_mut(&id(6)).map(|note| note.clone()),
            Some("Theirs".into())
        );
    }

    #[test]
    fn a_save_runs_once_until_its_result_lands() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "Bao".into();

        assert_eq!(state.begin_save(id(1), id(5)), Some(Some("Bao".into())));
        assert!(state.is_writing(&id(1), &id(5)));
        assert_eq!(
            state.begin_save(id(1), id(5)),
            None,
            "a second click must not pay for a second write"
        );
        assert!(!state.is_writing(&id(2), &id(5)), "guards are per owner");

        assert_eq!(
            state.finish_write(&id(1), &id(5)),
            Some(PendingWrite::Note(Some("Bao".into())))
        );
        assert!(!state.is_writing(&id(1), &id(5)));
        state.sync(&contact(5, Some("Bao")));
        assert!(
            !state.is_dirty(&id(5)),
            "the stored note is the draft's new starting point"
        );
    }

    #[test]
    fn a_failed_save_keeps_the_draft_and_can_be_retried() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "Bao".into();
        assert!(state.begin_save(id(1), id(5)).is_some());

        state.release_write(&id(1), &id(5));

        assert!(!state.is_writing(&id(1), &id(5)));
        assert!(state.is_dirty(&id(5)), "the unsaved edit stays");
        assert!(state.begin_save(id(1), id(5)).is_some());
    }

    #[test]
    fn there_is_nothing_to_save_without_a_draft_for_that_contact() {
        let mut state = ContactDetailState::default();
        assert_eq!(state.begin_save(id(1), id(5)), None);
        state.select(&contact(6, None));
        assert_eq!(state.begin_save(id(1), id(5)), None);
    }

    #[test]
    fn contact_info_running_write_survives_resets_until_its_own_result() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "First note".into();
        assert!(state.begin_save(id(1), id(5)).is_some());

        state.reset();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "Second note".into();
        assert_eq!(state.begin_save(id(1), id(5)), None);
        assert!(!state.begin_visibility_write(id(1), id(5)));
        assert_eq!(
            state.finish_write(&id(1), &id(5)),
            Some(PendingWrite::Note(Some("First note".into())))
        );
        assert_eq!(
            state.begin_save(id(1), id(5)),
            Some(Some("Second note".into()))
        );
    }

    #[test]
    fn reset_drops_the_selection_but_keeps_a_running_save() {
        let mut state = ContactDetailState::default();
        state.select(&contact(5, None));
        *state.note_mut(&id(5)).expect("draft") = "Bao".into();
        assert!(state.begin_save(id(1), id(5)).is_some());

        state.reset();

        assert_eq!(state.selected(), None);
        assert_eq!(state.note_mut(&id(5)), None);
        assert!(
            state.is_writing(&id(1), &id(5)),
            "a paid write still running keeps its guard"
        );
        assert_eq!(
            state.finish_write(&id(1), &id(5)),
            Some(PendingWrite::Note(Some("Bao".into())))
        );
    }
}
