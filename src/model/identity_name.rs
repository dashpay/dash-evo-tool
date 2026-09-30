//! Identity labels and ordered, session-local profile-name updates.

use dash_sdk::platform::Identifier;
use std::collections::HashMap;

/// Resolve a profile name, username, or shortened identity identifier.
pub fn display_label(display_name: Option<&str>, username: Option<&str>, id: &str) -> String {
    [display_name, username]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if id.trim().is_empty() {
                "Unknown identity".into()
            } else {
                shorten_id(id.trim())
            }
        })
}

/// Shorten a Base58 identifier while preserving both ends.
pub fn shorten_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 10 {
        return id.to_owned();
    }
    let head: String = chars[..5].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}…{tail}")
}

/// Profile names accepted during this network session.
#[derive(Debug, Default)]
pub(crate) struct ProfileNames {
    entries: HashMap<Identifier, ProfileName>,
}

#[derive(Debug, Default)]
struct ProfileName {
    revision: u64,
    accepted: Option<Option<String>>,
}

impl ProfileNames {
    pub(crate) fn get(&self, id: Identifier) -> Option<Option<String>> {
        self.entries
            .get(&id)
            .and_then(|entry| entry.accepted.clone())
    }

    pub(crate) fn begin_load(&mut self, id: Identifier) -> u64 {
        let entry = self.entries.entry(id).or_default();
        entry.revision += 1;
        entry.revision
    }

    pub(crate) fn loaded(&mut self, id: Identifier, revision: u64, name: Option<&str>) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.revision != revision {
            return false;
        }
        entry.accepted = Some(clean_name(name));
        true
    }

    pub(crate) fn saved(&mut self, id: Identifier, name: Option<&str>) {
        let revision = self.begin_load(id);
        self.loaded(id, revision, name);
    }
}

fn clean_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_precedes_username_and_blank_names_fall_back() {
        assert_eq!(
            display_label(Some(" Alex "), Some("alex.dash"), "abcdefghijk"),
            "Alex"
        );
        assert_eq!(
            display_label(Some(" "), Some("alex.dash"), "abcdefghijk"),
            "alex.dash"
        );
        assert_eq!(display_label(None, None, "abcdefghijk"), "abcde…ijk");
    }

    #[test]
    fn late_load_cannot_replace_saved_or_cleared_profile_name() {
        let id = Identifier::from([1; 32]);
        let mut names = ProfileNames::default();
        let load = names.begin_load(id);
        names.saved(id, Some("New name"));
        assert!(!names.loaded(id, load, Some("Old name")));
        assert_eq!(names.get(id), Some(Some("New name".into())));
        let load = names.begin_load(id);
        names.saved(id, None);
        assert!(!names.loaded(id, load, Some("Old name")));
        assert_eq!(names.get(id), Some(None));
    }

    #[test]
    fn latest_load_wins_and_identity_names_are_isolated() {
        let id = Identifier::from([1; 32]);
        let other = Identifier::from([2; 32]);
        let mut names = ProfileNames::default();
        let old = names.begin_load(id);
        let new = names.begin_load(id);
        assert!(!names.loaded(id, old, Some("Old")));
        assert!(names.loaded(id, new, Some("New")));
        assert_eq!(names.get(other), None);
        assert_eq!(ProfileNames::default().get(id), None);
    }
}
