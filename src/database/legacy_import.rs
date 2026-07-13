//! Typed readers for the non-wallet legacy `data.db` tables.
//!
//! The unwire moved user preferences, scheduled DPNS votes and top-up
//! history out of `data.db` into the upstream k/v store, but left the old
//! rows in place. These readers are the read half of the import that
//! carries them across; they decode rows into the modern domain types and
//! report per-row decode failures as counters. Deciding what to do with a
//! failure (retry, surface a banner) is the caller's job — see
//! [`backend_task::migration`](crate::backend_task::migration).
//!
//! Every reader treats a missing table as "no data": a fresh install never
//! creates these tables, and that is not an error.

use std::collections::BTreeMap;

use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rusqlite::Connection;

use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::database::{Database, column_exists, table_exists};
use crate::model::settings::{
    AppSettings, RootScreenType, UserMode, network_from_legacy_str, theme_mode_from_str,
};

/// Legacy spelling of mainnet in `data.db`. Migration 29 rewrites it to
/// `mainnet`, but a DB that never reached v29 still carries the old value,
/// so every network filter accepts both spellings.
const LEGACY_MAINNET_ALIAS: &str = "dash";

/// Outcome of one legacy scheduled-vote read.
///
/// `unreadable` counts rows the reader could not decode into a
/// [`ScheduledDPNSVote`] (corrupt voter id, unparseable vote choice). They
/// are reported rather than silently dropped so the caller can refuse to
/// mark the import complete — a dropped vote is a missed vote window.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LegacyScheduledVotes {
    /// Rows decoded into the modern domain type.
    pub votes: Vec<ScheduledDPNSVote>,
    /// Rows that failed to decode. Never silently ignored by the caller.
    pub unreadable: u32,
}

/// Per-identity top-up history keyed by identity id, mirroring the shape
/// [`AppContext::save_top_ups`](crate::context::AppContext::save_top_ups)
/// persists (`top_up_index -> amount`).
pub(crate) type LegacyTopUps = Vec<([u8; 32], BTreeMap<u32, u64>)>;

/// Read the user preferences held in the legacy `settings` row.
///
/// Returns `None` when the table or its singleton row is absent (a fresh
/// install). Columns the row does not carry — older schemas predate several
/// of them — fall back to the corresponding [`AppSettings::default`] value,
/// so a partial legacy row still yields a complete settings blob.
pub(crate) fn read_app_settings(conn: &Connection) -> rusqlite::Result<Option<AppSettings>> {
    if !table_exists(conn, "settings")? {
        return Ok(None);
    }

    // `None` means "autodetect at load time", which is what a legacy row with
    // no explicit path means too. `default()` autodetects eagerly, so clear it
    // — otherwise the import would freeze today's detected path into the blob.
    let mut settings = AppSettings {
        dash_qt_path: None,
        ..AppSettings::default()
    };

    // Every column below is probed: the legacy ladder added them one at a
    // time, so an old `data.db` legitimately lacks the later ones. A missing
    // column keeps the default rather than failing the whole read.
    let columns = settings_columns(conn)?;
    let fields: Vec<&str> = [
        "network",
        "start_root_screen",
        "theme_preference",
        "onboarding_completed",
        "show_evonode_tools",
        "user_mode",
        "custom_dash_qt_path",
        "overwrite_dash_conf",
        "disable_zmq",
        "auto_start_spv",
        "close_dash_qt_on_exit",
    ]
    .into_iter()
    .filter(|col| columns.iter().any(|c| c == col))
    .collect();
    if fields.is_empty() {
        return Ok(None);
    }

    // Static column names from the list above — no user input reaches this
    // `format!`. SQLite cannot bind identifiers, so this is the canonical shape.
    let sql = format!("SELECT {} FROM settings WHERE id = 1", fields.join(", "));
    let row = conn.query_row(&sql, [], |row| {
        let mut values: BTreeMap<&str, rusqlite::types::Value> = BTreeMap::new();
        for (idx, name) in fields.iter().enumerate() {
            values.insert(name, row.get::<_, rusqlite::types::Value>(idx)?);
        }
        Ok(values)
    });

    let values = match row {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e),
    };

    if let Some(s) = value_as_string(&values, "network")
        && let Some(network) = network_from_legacy_str(&s)
    {
        settings.network = network;
    }
    if let Some(i) = value_as_i64(&values, "start_root_screen")
        && let Ok(i) = u32::try_from(i)
        && let Some(root) = RootScreenType::from_int(i)
    {
        settings.root_screen_type = root;
    }
    if let Some(s) = value_as_string(&values, "theme_preference") {
        settings.theme_mode = theme_mode_from_str(&s);
    }
    if let Some(s) = value_as_string(&values, "user_mode") {
        settings.user_mode = UserMode::from_str_or_default(&s);
    }
    if let Some(b) = value_as_bool(&values, "onboarding_completed") {
        settings.onboarding_completed = b;
    }
    if let Some(b) = value_as_bool(&values, "show_evonode_tools") {
        settings.show_evonode_tools = b;
    }
    if let Some(b) = value_as_bool(&values, "overwrite_dash_conf") {
        settings.overwrite_dash_conf = b;
    }
    if let Some(b) = value_as_bool(&values, "disable_zmq") {
        settings.disable_zmq = b;
    }
    if let Some(b) = value_as_bool(&values, "auto_start_spv") {
        settings.auto_start_spv = b;
    }
    if let Some(b) = value_as_bool(&values, "close_dash_qt_on_exit") {
        settings.close_dash_qt_on_exit = b;
    }
    if let Some(s) = value_as_string(&values, "custom_dash_qt_path") {
        settings.dash_qt_path = Some(std::path::PathBuf::from(s));
    }

    Ok(Some(settings))
}

/// Read the scheduled DPNS votes queued for `network`.
///
/// Pre-v6 schemas have no `network` column; those rows predate multi-network
/// support and belong to mainnet, so they are returned only for
/// [`Network::Mainnet`].
pub(crate) fn read_scheduled_votes(
    conn: &Connection,
    network: Network,
) -> rusqlite::Result<LegacyScheduledVotes> {
    if !table_exists(conn, "scheduled_votes")? {
        return Ok(LegacyScheduledVotes::default());
    }

    let has_network = column_exists(conn, "scheduled_votes", "network")?;
    let base = "SELECT identity_id, contested_name, vote_choice, time, executed \
                FROM scheduled_votes";

    let mut out = LegacyScheduledVotes::default();
    let mut stmt;
    let rows = if has_network {
        stmt = conn.prepare(&format!("{base} WHERE network IN (?1, ?2)"))?;
        stmt.query(rusqlite::params![
            network.to_string(),
            mainnet_alias_for(network)
        ])?
    } else if network == Network::Mainnet {
        stmt = conn.prepare(base)?;
        stmt.query([])?
    } else {
        return Ok(out);
    };

    let mut rows = rows;
    while let Some(row) = rows.next()? {
        let voter_id: Vec<u8> = row.get(0)?;
        let contested_name: String = row.get(1)?;
        let vote_choice: String = row.get(2)?;
        let unix_timestamp: u64 = row.get(3)?;
        let executed: i64 = row.get(4)?;

        let Ok(voter_id) = Identifier::from_bytes(&voter_id) else {
            tracing::warn!(
                target = "database::legacy_import",
                contested_name = %contested_name,
                "Skipping legacy scheduled vote with an undecodable voter id",
            );
            out.unreadable = out.unreadable.saturating_add(1);
            continue;
        };
        let Some(choice) = parse_vote_choice(&vote_choice) else {
            tracing::warn!(
                target = "database::legacy_import",
                contested_name = %contested_name,
                "Skipping legacy scheduled vote with an unparseable vote choice",
            );
            out.unreadable = out.unreadable.saturating_add(1);
            continue;
        };

        out.votes.push(ScheduledDPNSVote {
            contested_name,
            voter_id,
            choice,
            unix_timestamp,
            executed_successfully: executed != 0,
        });
    }

    Ok(out)
}

/// Read the top-up history of every identity on `network`.
///
/// The legacy `top_up` table carries no network column, so rows are scoped
/// by joining the legacy `identity` table. An identity whose row is gone
/// contributes nothing — its top-ups are unreachable audit trail.
pub(crate) fn read_top_ups(conn: &Connection, network: Network) -> rusqlite::Result<LegacyTopUps> {
    if !table_exists(conn, "top_up")? || !table_exists(conn, "identity")? {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT t.identity_id, t.top_up_index, t.amount \
         FROM top_up t JOIN identity i ON i.id = t.identity_id \
         WHERE i.network IN (?1, ?2) \
         ORDER BY t.identity_id, t.top_up_index",
    )?;
    let mut rows = stmt.query(rusqlite::params![
        network.to_string(),
        mainnet_alias_for(network)
    ])?;

    let mut grouped: BTreeMap<[u8; 32], BTreeMap<u32, u64>> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let identity_id: Vec<u8> = row.get(0)?;
        let index: u32 = row.get(1)?;
        let amount: u64 = row.get(2)?;
        let Ok(identity_id) = <[u8; 32]>::try_from(identity_id.as_slice()) else {
            tracing::warn!(
                target = "database::legacy_import",
                blob_len = identity_id.len(),
                "Skipping legacy top-up row with a non-32-byte identity id",
            );
            continue;
        };
        grouped
            .entry(identity_id)
            .or_default()
            .insert(index, amount);
    }

    Ok(grouped.into_iter().collect())
}

/// Decode the legacy `vote_choice` text, which is the `Display` form of
/// [`ResourceVoteChoice`] (`Abstain`, `Lock`, `TowardsIdentity(<base58>)`).
fn parse_vote_choice(raw: &str) -> Option<ResourceVoteChoice> {
    match raw {
        "Abstain" => Some(ResourceVoteChoice::Abstain),
        "Lock" => Some(ResourceVoteChoice::Lock),
        other => {
            let inner = other
                .strip_prefix("TowardsIdentity(")
                .and_then(|s| s.strip_suffix(')'))?;
            Identifier::from_string(inner, Encoding::Base58)
                .ok()
                .map(ResourceVoteChoice::TowardsIdentity)
        }
    }
}

/// The network spelling a pre-v29 `data.db` used. Only mainnet was ever
/// renamed, so every other network maps to itself and the two-value `IN`
/// filter stays a single code path.
fn mainnet_alias_for(network: Network) -> String {
    match network {
        Network::Mainnet => LEGACY_MAINNET_ALIAS.to_string(),
        other => other.to_string(),
    }
}

fn settings_columns(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('settings')")?;
    let names = stmt.query_map([], |row| row.get::<_, String>(0))?;
    names.collect()
}

fn value_as_string(values: &BTreeMap<&str, rusqlite::types::Value>, key: &str) -> Option<String> {
    match values.get(key) {
        Some(rusqlite::types::Value::Text(s)) => Some(s.clone()),
        _ => None,
    }
}

fn value_as_i64(values: &BTreeMap<&str, rusqlite::types::Value>, key: &str) -> Option<i64> {
    match values.get(key) {
        Some(rusqlite::types::Value::Integer(i)) => Some(*i),
        _ => None,
    }
}

fn value_as_bool(values: &BTreeMap<&str, rusqlite::types::Value>, key: &str) -> Option<bool> {
    value_as_i64(values, key).map(|i| i != 0)
}

impl Database {
    /// Read the legacy `settings` row through the shared connection.
    /// See [`read_app_settings`].
    pub(crate) fn read_legacy_app_settings(&self) -> rusqlite::Result<Option<AppSettings>> {
        read_app_settings(&self.locked_conn())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::settings::ThemeMode;

    /// The v0.10-dev `settings` shape: every user-preference column the
    /// ladder ever added, at the post-v29 (`mainnet`) network spelling.
    fn create_settings_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                network TEXT NOT NULL,
                start_root_screen INTEGER NOT NULL,
                custom_dash_qt_path TEXT,
                overwrite_dash_conf INTEGER,
                theme_preference TEXT,
                disable_zmq INTEGER,
                onboarding_completed INTEGER,
                show_evonode_tools INTEGER,
                user_mode TEXT,
                auto_start_spv INTEGER,
                close_dash_qt_on_exit INTEGER,
                database_version INTEGER NOT NULL
            );",
        )
        .unwrap();
    }

    fn create_scheduled_votes_table(conn: &Connection, with_network: bool) {
        let network_col = if with_network {
            ", network TEXT NOT NULL"
        } else {
            ""
        };
        conn.execute_batch(&format!(
            "CREATE TABLE scheduled_votes (
                identity_id BLOB NOT NULL,
                contested_name TEXT NOT NULL,
                vote_choice TEXT NOT NULL,
                time INTEGER NOT NULL,
                executed INTEGER NOT NULL DEFAULT 0
                {network_col},
                PRIMARY KEY (identity_id, contested_name)
            );"
        ))
        .unwrap();
    }

    fn insert_vote(
        conn: &Connection,
        voter: &[u8; 32],
        name: &str,
        choice: &str,
        executed: i64,
        network: Option<&str>,
    ) {
        match network {
            Some(net) => conn
                .execute(
                    "INSERT INTO scheduled_votes
                     (identity_id, contested_name, vote_choice, time, executed, network)
                     VALUES (?1, ?2, ?3, 1700000000, ?4, ?5)",
                    rusqlite::params![voter.as_slice(), name, choice, executed, net],
                )
                .unwrap(),
            None => conn
                .execute(
                    "INSERT INTO scheduled_votes
                     (identity_id, contested_name, vote_choice, time, executed)
                     VALUES (?1, ?2, ?3, 1700000000, ?4)",
                    rusqlite::params![voter.as_slice(), name, choice, executed],
                )
                .unwrap(),
        };
    }

    /// A user on testnet with a customised theme, root screen and toggles
    /// keeps every one of them across the upgrade. The network field is the
    /// safety-critical one: dropping it relaunches a testnet user on mainnet.
    #[test]
    fn app_settings_round_trip_every_preference() {
        let conn = Connection::open_in_memory().unwrap();
        create_settings_table(&conn);
        conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, custom_dash_qt_path,
                overwrite_dash_conf, theme_preference, disable_zmq, onboarding_completed,
                show_evonode_tools, user_mode, auto_start_spv, close_dash_qt_on_exit,
                database_version)
             VALUES (1, 'testnet', ?1, '/opt/dash-qt', 0, 'Dark', 1, 1, 1, 'Beginner', 0, 0, 40)",
            rusqlite::params![RootScreenType::RootScreenDPNSScheduledVotes.to_int()],
        )
        .unwrap();

        let settings = read_app_settings(&conn).unwrap().expect("settings row");

        assert_eq!(settings.network, Network::Testnet);
        assert_eq!(
            settings.root_screen_type,
            RootScreenType::RootScreenDPNSScheduledVotes
        );
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        assert!(settings.onboarding_completed);
        assert!(settings.show_evonode_tools);
        assert_eq!(settings.user_mode, UserMode::Beginner);
        assert!(settings.disable_zmq);
        assert!(!settings.overwrite_dash_conf);
        assert!(!settings.auto_start_spv);
        assert!(!settings.close_dash_qt_on_exit);
        assert_eq!(
            settings.dash_qt_path,
            Some(std::path::PathBuf::from("/opt/dash-qt"))
        );
    }

    /// A `data.db` that never reached migration 29 still spells mainnet
    /// `dash`. Failing to map it would silently relaunch the user on the
    /// default network.
    #[test]
    fn app_settings_accepts_pre_v29_dash_network_name() {
        let conn = Connection::open_in_memory().unwrap();
        create_settings_table(&conn);
        conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, 'dash', 0, 29)",
            [],
        )
        .unwrap();

        let settings = read_app_settings(&conn).unwrap().expect("settings row");
        assert_eq!(settings.network, Network::Mainnet);
    }

    /// The v0.9.0 schema has neither `theme_preference` nor the onboarding
    /// columns. Absent columns must fall back to defaults, not fail the read.
    #[test]
    fn app_settings_missing_columns_fall_back_to_defaults() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                network TEXT NOT NULL,
                start_root_screen INTEGER NOT NULL,
                database_version INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, 'testnet', 0, 5)",
            [],
        )
        .unwrap();

        let settings = read_app_settings(&conn).unwrap().expect("settings row");
        let defaults = AppSettings::default();
        assert_eq!(settings.network, Network::Testnet, "present column wins");
        assert_eq!(settings.theme_mode, defaults.theme_mode);
        assert_eq!(settings.onboarding_completed, defaults.onboarding_completed);
    }

    /// A legacy row with no explicit Dash-Qt path means "autodetect", so the
    /// import must leave the field empty rather than freezing today's detected
    /// path into storage — otherwise moving Dash-Qt later breaks the link.
    #[test]
    fn app_settings_null_dash_qt_path_stays_autodetect() {
        let conn = Connection::open_in_memory().unwrap();
        create_settings_table(&conn);
        conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, custom_dash_qt_path,
                database_version)
             VALUES (1, 'testnet', 0, NULL, 40)",
            [],
        )
        .unwrap();

        let settings = read_app_settings(&conn).unwrap().expect("settings row");
        assert_eq!(settings.dash_qt_path, None);
    }

    /// A fresh install has no `settings` table at all.
    #[test]
    fn app_settings_absent_table_reads_none() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_app_settings(&conn).unwrap().is_none());
    }

    /// An empty `settings` table (table created, row never written) is also
    /// "no legacy settings" — not an error.
    #[test]
    fn app_settings_absent_row_reads_none() {
        let conn = Connection::open_in_memory().unwrap();
        create_settings_table(&conn);
        assert!(read_app_settings(&conn).unwrap().is_none());
    }

    /// Every vote choice the UI can queue must survive the import, including
    /// the executed flag — a re-imported vote that lost `executed` would be
    /// cast twice.
    #[test]
    fn scheduled_votes_decode_all_choice_kinds() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, true);
        let voter = [0x11u8; 32];
        let towards = Identifier::from([0x22u8; 32]);

        insert_vote(&conn, &voter, "alice", "Abstain", 0, Some("testnet"));
        insert_vote(&conn, &voter, "bob", "Lock", 1, Some("testnet"));
        insert_vote(
            &conn,
            &voter,
            "carol",
            &format!("TowardsIdentity({})", towards.to_string(Encoding::Base58)),
            0,
            Some("testnet"),
        );

        let read = read_scheduled_votes(&conn, Network::Testnet).unwrap();

        assert_eq!(read.unreadable, 0);
        assert_eq!(read.votes.len(), 3);
        let by_name = |name: &str| {
            read.votes
                .iter()
                .find(|v| v.contested_name == name)
                .unwrap_or_else(|| panic!("vote {name} missing"))
        };
        assert_eq!(by_name("alice").choice, ResourceVoteChoice::Abstain);
        assert_eq!(by_name("bob").choice, ResourceVoteChoice::Lock);
        assert!(by_name("bob").executed_successfully);
        assert!(!by_name("alice").executed_successfully);
        assert_eq!(
            by_name("carol").choice,
            ResourceVoteChoice::TowardsIdentity(towards)
        );
        assert_eq!(by_name("alice").voter_id, Identifier::from(voter));
        assert_eq!(by_name("alice").unix_timestamp, 1_700_000_000);
    }

    /// Votes queued on another network must not leak into this network's
    /// import — casting a mainnet vote from a testnet queue is a real error.
    #[test]
    fn scheduled_votes_filter_by_network() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, true);
        let voter = [0x11u8; 32];
        insert_vote(&conn, &voter, "on-testnet", "Lock", 0, Some("testnet"));
        insert_vote(&conn, &voter, "on-mainnet", "Lock", 0, Some("mainnet"));

        let testnet = read_scheduled_votes(&conn, Network::Testnet).unwrap();
        assert_eq!(testnet.votes.len(), 1);
        assert_eq!(testnet.votes[0].contested_name, "on-testnet");

        let mainnet = read_scheduled_votes(&conn, Network::Mainnet).unwrap();
        assert_eq!(mainnet.votes.len(), 1);
        assert_eq!(mainnet.votes[0].contested_name, "on-mainnet");
    }

    /// Pre-v29 mainnet rows spell the network `dash`; they belong to the
    /// mainnet queue.
    #[test]
    fn scheduled_votes_accept_pre_v29_dash_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, true);
        insert_vote(&conn, &[0x11u8; 32], "legacy", "Lock", 0, Some("dash"));

        let read = read_scheduled_votes(&conn, Network::Mainnet).unwrap();
        assert_eq!(read.votes.len(), 1);
        assert!(
            read_scheduled_votes(&conn, Network::Testnet)
                .unwrap()
                .votes
                .is_empty(),
            "a pre-v29 mainnet row must not surface on testnet",
        );
    }

    /// The v0.9.0 `scheduled_votes` shape predates multi-network support and
    /// has no `network` column; those rows are mainnet's.
    #[test]
    fn scheduled_votes_without_network_column_belong_to_mainnet() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, false);
        insert_vote(&conn, &[0x11u8; 32], "pre-v6", "Abstain", 0, None);

        let mainnet = read_scheduled_votes(&conn, Network::Mainnet).unwrap();
        assert_eq!(mainnet.votes.len(), 1);
        assert_eq!(mainnet.votes[0].contested_name, "pre-v6");

        let testnet = read_scheduled_votes(&conn, Network::Testnet).unwrap();
        assert!(testnet.votes.is_empty());
    }

    /// A corrupt row is counted, never silently dropped, and never blocks the
    /// readable votes around it.
    #[test]
    fn scheduled_votes_count_unreadable_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, true);
        let voter = [0x11u8; 32];
        insert_vote(&conn, &voter, "good", "Lock", 0, Some("testnet"));
        insert_vote(&conn, &voter, "bad-choice", "Nonsense", 0, Some("testnet"));
        conn.execute(
            "INSERT INTO scheduled_votes
             (identity_id, contested_name, vote_choice, time, executed, network)
             VALUES (?1, 'bad-voter', 'Lock', 1, 0, 'testnet')",
            rusqlite::params![vec![0xFFu8; 5]],
        )
        .unwrap();

        let read = read_scheduled_votes(&conn, Network::Testnet).unwrap();
        assert_eq!(read.votes.len(), 1, "the readable vote still imports");
        assert_eq!(read.votes[0].contested_name, "good");
        assert_eq!(read.unreadable, 2, "both corrupt rows are reported");
    }

    #[test]
    fn scheduled_votes_absent_table_reads_empty() {
        let conn = Connection::open_in_memory().unwrap();
        let read = read_scheduled_votes(&conn, Network::Testnet).unwrap();
        assert_eq!(read, LegacyScheduledVotes::default());
    }

    /// Top-ups group per identity and stay scoped to the identity's network.
    #[test]
    fn top_ups_group_per_identity_and_scope_by_network() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE identity (id BLOB PRIMARY KEY, network TEXT NOT NULL);
             CREATE TABLE top_up (
                identity_id BLOB NOT NULL,
                top_up_index INTEGER NOT NULL,
                amount INTEGER NOT NULL,
                PRIMARY KEY (identity_id, top_up_index)
             );",
        )
        .unwrap();
        let mine = [0xAAu8; 32];
        let other_net = [0xBBu8; 32];
        for (id, net) in [(mine, "testnet"), (other_net, "mainnet")] {
            conn.execute(
                "INSERT INTO identity (id, network) VALUES (?1, ?2)",
                rusqlite::params![id.as_slice(), net],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES
                (?1, 0, 1000), (?1, 1, 2000), (?2, 0, 9999)",
            rusqlite::params![mine.as_slice(), other_net.as_slice()],
        )
        .unwrap();

        let top_ups = read_top_ups(&conn, Network::Testnet).unwrap();

        assert_eq!(top_ups.len(), 1, "only the testnet identity's top-ups");
        assert_eq!(top_ups[0].0, mine);
        assert_eq!(top_ups[0].1, BTreeMap::from([(0, 1000), (1, 2000)]));
    }

    #[test]
    fn top_ups_absent_table_reads_empty() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_top_ups(&conn, Network::Testnet).unwrap().is_empty());
    }
}
