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
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use rusqlite::Connection;

use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::database::{Database, column_exists, table_exists};
use crate::model::qualified_identity::{IdentityStatus, QualifiedIdentity};
use crate::model::settings::{
    AppSettings, RootScreenType, network_from_legacy_str, theme_mode_from_str,
};
use crate::model::user_role::UserRole;

/// Legacy spelling of mainnet in `data.db`. Migration 29 rewrites it to
/// `mainnet`, but a DB that never reached v29 still carries the old value,
/// so every network filter accepts both spellings.
const LEGACY_MAINNET_ALIAS: &str = "dash";

/// Outcome of one legacy scheduled-vote read.
///
/// `unreadable` counts rows the reader could not decode into a
/// [`ScheduledDPNSVote`] — a malformed column (NULL, wrong type, out-of-range
/// integer) as much as a corrupt voter id or an unparseable vote choice. They
/// are reported rather than silently dropped so the caller can refuse to
/// mark the import complete — a dropped vote is a missed vote window.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LegacyScheduledVotes {
    /// Rows decoded into the modern domain type.
    pub votes: Vec<ScheduledDPNSVote>,
    /// Rows that failed to decode. Never silently ignored by the caller.
    pub unreadable: u32,
}

/// Outcome of one legacy top-up-history read.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LegacyTopUps {
    /// Per-identity history keyed by identity id.
    pub top_ups: Vec<([u8; 32], BTreeMap<u32, u64>)>,
    /// Rows that failed to decode. Never silently ignored by the caller.
    pub unreadable: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("the saved network setting is not recognized")]
struct InvalidLegacyNetwork;

/// One decoded local identity from the legacy `identity` table.
///
/// `qi` carries the identity's private keys, so it is moved straight into
/// [`AppContext::insert_local_qualified_identity`](crate::context::AppContext::insert_local_qualified_identity)
/// — the only writer that routes key material through the vault seam.
#[derive(Debug)]
pub(crate) struct LegacyIdentityRow {
    /// 32-byte identity id, the k/v scope key.
    pub id: [u8; 32],
    /// The decoded identity, with `status` and `network` already restored
    /// from their columns — the bincode blob carries neither.
    pub qi: QualifiedIdentity,
    /// `(wallet seed hash, account index)` of the owning wallet. `None` for a
    /// wallet-less identity (a masternode loaded by ProTxHash). Both-or-neither,
    /// enforced by the legacy `CHECK` constraint.
    pub wallet: Option<([u8; 32], u32)>,
}

/// Outcome of looking one identity up in the legacy `identity` table.
///
/// Separates "no such user identity here" — a missing table, a missing row, an
/// observed-identity cache row, a NULL blob — from "the row is there but will
/// not decode". The first is the ordinary answer on any install with nothing to
/// recover; the second is corruption a caller must surface as a typed error
/// rather than silently treat as an empty record.
#[derive(Debug)]
pub(crate) enum LegacyIdentityLookup {
    /// A local identity row that decoded cleanly. Boxed because a decoded row
    /// dwarfs the two "nothing here" answers, which are the common ones.
    Found(Box<LegacyIdentityRow>),
    /// No local, non-NULL row for this id on this network.
    Absent,
    /// The row exists but its columns or blob could not be decoded.
    Unreadable,
}

/// Outcome of one legacy identity read.
///
/// `unreadable` counts rows whose blob could not be decoded. The caller records
/// them as a durable warning and leaves them in the legacy file — never deleted,
/// so a build with a fixed decoder can still recover them on an explicit
/// re-import. The import itself completes: re-running it on every launch would
/// resurrect identities the user has deliberately deleted.
#[derive(Debug, Default)]
pub(crate) struct LegacyIdentities {
    /// Rows decoded into the modern domain type.
    pub identities: Vec<LegacyIdentityRow>,
    /// Rows that failed to decode. Never silently dropped by the caller.
    pub unreadable: u32,
}

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

    if let Some(s) = value_as_string(&values, "network") {
        settings.network = network_from_legacy_str(&s).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(InvalidLegacyNetwork),
            )
        })?;
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
    // The legacy `user_mode` column only ever held the retired `UserMode`
    // strings, which gated nothing and so carry no role information. They decode
    // to `None` — no role was ever chosen — and the app resolves that to
    // `UserRole::WHEN_UNSET`, the tier the legacy build exposed unconditionally.
    if let Some(s) = value_as_string(&values, "user_mode") {
        settings.user_role = UserRole::from_persisted(&s);
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
        // Column decoding is per-row, like the domain decoding below it: a NULL,
        // a type mismatch or an out-of-range integer (a negative `time` fails the
        // `u64` range check) is corruption of ONE row. Propagating it would
        // discard every vote already read in this pass and turn a
        // warning-and-skip into a hard migration failure.
        let decoded = decode_scheduled_vote_columns(row);
        let (voter_id, contested_name, vote_choice, unix_timestamp, executed) = match decoded {
            Ok(columns) => columns,
            Err(e) => {
                tracing::warn!(
                    target = "database::legacy_import",
                    error = ?e,
                    "Skipping legacy scheduled vote with an unreadable column",
                );
                out.unreadable = out.unreadable.saturating_add(1);
                continue;
            }
        };

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

/// Read the local identities — and the private keys they hold — for `network`.
///
/// Only `is_local = 1` rows with a non-NULL `data` blob are user identities.
/// v0.9.3 also cached observed identities (`is_local = 0`, often a NULL blob);
/// every one of its own read paths filtered those out, and so does this one —
/// they are a lookup cache, not the user's data.
///
/// A row whose blob will not decode is counted in
/// [`LegacyIdentities::unreadable`] and skipped, so one bad blob never blocks
/// the identities around it.
///
/// A skipped row is logged by its identity id alone — the public, on-chain
/// handle, which is what lets the user tell which identity did not come across.
/// The `data` blob and everything it decodes to (private keys above all) are
/// never logged, at any level.
pub(crate) fn read_identities(
    conn: &Connection,
    network: Network,
) -> rusqlite::Result<LegacyIdentities> {
    if !table_exists(conn, "identity")? {
        return Ok(LegacyIdentities::default());
    }

    let mut stmt = conn.prepare(LOCAL_IDENTITY_SELECT)?;
    let mut rows = stmt.query(rusqlite::params![
        network.to_string(),
        mainnet_alias_for(network)
    ])?;

    let mut out = LegacyIdentities::default();
    while let Some(row) = rows.next()? {
        match decode_identity_row(row, network) {
            Some(decoded) => out.identities.push(decoded),
            None => out.unreadable = out.unreadable.saturating_add(1),
        }
    }

    Ok(out)
}

/// Read the one local identity stored under `id` on `network`.
///
/// The single-row twin of [`read_identities`], same filter and same decoder.
/// Every ordinary "not here" answer is [`LegacyIdentityLookup::Absent`]; only a
/// row that will not decode is [`LegacyIdentityLookup::Unreadable`], which a
/// caller must never read as an empty record.
pub(crate) fn read_identity_row(
    conn: &Connection,
    network: Network,
    id: &[u8; 32],
) -> rusqlite::Result<LegacyIdentityLookup> {
    if !table_exists(conn, "identity")? {
        return Ok(LegacyIdentityLookup::Absent);
    }

    let mut stmt = conn.prepare(&format!("{LOCAL_IDENTITY_SELECT} AND id = ?3"))?;
    let mut rows = stmt.query(rusqlite::params![
        network.to_string(),
        mainnet_alias_for(network),
        id.as_slice()
    ])?;

    let Some(row) = rows.next()? else {
        return Ok(LegacyIdentityLookup::Absent);
    };
    Ok(match decode_identity_row(row, network) {
        Some(decoded) => LegacyIdentityLookup::Found(Box::new(decoded)),
        None => LegacyIdentityLookup::Unreadable,
    })
}

/// Whether the legacy `identity` table holds any local identity for `network`.
///
/// The cheapest question a caller can ask before offering anything built on
/// this table: it stops at the first matching row and answers `false` on a
/// fresh install, whose schema never creates the table at all. Same filter as
/// [`read_identities`], so "there is something here" and "here it is" cannot
/// disagree about which rows count.
pub(crate) fn local_identities_exist(
    conn: &Connection,
    network: Network,
) -> rusqlite::Result<bool> {
    if !table_exists(conn, "identity")? {
        return Ok(false);
    }

    conn.query_row(
        &format!("SELECT EXISTS({LOCAL_IDENTITY_SELECT})"),
        rusqlite::params![network.to_string(), mainnet_alias_for(network)],
        |row| row.get::<_, i64>(0).map(|found| found != 0),
    )
}

/// The rows of the legacy `identity` table that hold a user's own identity.
/// Both readers bind `?1`/`?2` to the two accepted network spellings, so
/// neither can drift from the other's idea of what belongs to this network.
const LOCAL_IDENTITY_SELECT: &str = "SELECT id, data, status, wallet, wallet_index, alias FROM identity \
     WHERE is_local = 1 AND data IS NOT NULL AND network IN (?1, ?2)";

/// Decode one legacy `identity` row, or `None` when it is corrupt — the one
/// decoder behind both readers, so they cannot disagree about which rows are
/// readable. Each rejection below logs its own reason; a rejected row is named
/// by its identity id alone, never by anything the blob decodes to.
fn decode_identity_row(row: &rusqlite::Row<'_>, network: Network) -> Option<LegacyIdentityRow> {
    // A wrong SQLite storage class on any column is row-level corruption:
    // decode through a `Result` so a bad column costs its own row, not the
    // whole read. A bare `row.get::<_>?` here would escape `read_identities`
    // and discard every identity already accumulated this pass — every other
    // corruption case below is counted and skipped, and this must match.
    let (id, data, status, wallet, wallet_index, alias) = match decode_identity_columns(row) {
        Ok(columns) => columns,
        Err(_) => {
            // Not even the id is logged here: the failing column may be the id
            // itself, so there is no trustworthy handle to name the row by.
            tracing::warn!(
                target = "database::legacy_import",
                "Skipping legacy identity whose column types could not be read",
            );
            return None;
        }
    };

    let Ok(id) = <[u8; 32]>::try_from(id.as_slice()) else {
        tracing::warn!(
            target = "database::legacy_import",
            blob_len = id.len(),
            "Skipping legacy identity with a non-32-byte id",
        );
        return None;
    };

    let Ok(status) = u8::try_from(status) else {
        tracing::warn!(
            target = "database::legacy_import",
            identity = %hex::encode(id),
            "Skipping legacy identity with an out-of-range status value",
        );
        return None;
    };

    let wallet_index = match wallet_index.map(u32::try_from) {
        None => None,
        Some(Ok(index)) => Some(index),
        Some(Err(_)) => {
            tracing::warn!(
                target = "database::legacy_import",
                identity = %hex::encode(id),
                "Skipping legacy identity with an out-of-range wallet index",
            );
            return None;
        }
    };

    // Both-or-neither: the legacy `CHECK` guarantees it, so a half-filled
    // link is corruption. Dropping just the link would silently orphan the
    // identity from the wallet that owns its keys, so the row is reported
    // instead — the sentinel stays open and a later build can retry it.
    let wallet = match (wallet, wallet_index) {
        (None, None) => None,
        (Some(seed_hash), Some(index)) => match <[u8; 32]>::try_from(seed_hash.as_slice()) {
            Ok(seed_hash) => Some((seed_hash, index)),
            Err(_) => {
                tracing::warn!(
                    target = "database::legacy_import",
                    identity = %hex::encode(id),
                    "Skipping legacy identity whose wallet link is not a 32-byte seed hash",
                );
                return None;
            }
        },
        _ => {
            tracing::warn!(
                target = "database::legacy_import",
                identity = %hex::encode(id),
                "Skipping legacy identity with a half-filled wallet link",
            );
            return None;
        }
    };

    let Ok(mut qi) = QualifiedIdentity::from_bytes(&data) else {
        tracing::warn!(
            target = "database::legacy_import",
            identity = %hex::encode(id),
            "Skipping legacy identity whose stored data could not be decoded",
        );
        return None;
    };

    // The vault key derives from the id inside the blob, not this row's `id`
    // column (see `insert_local_qualified_identity`), while the migration's
    // skip-if-present precheck keys off the column. A hand-edited row whose
    // two ids disagree would pass that precheck and then silently overwrite a
    // different, already-loaded identity. Treat the divergence as row-level
    // corruption: count it and skip, like every other bad row here.
    if qi.identity.id().to_buffer() != id {
        tracing::warn!(
            target = "database::legacy_import",
            identity = %hex::encode(id),
            embedded = %hex::encode(qi.identity.id().to_buffer()),
            "Skipping legacy identity whose row id and stored id disagree",
        );
        return None;
    }

    // Neither field is in the bincode blob — the legacy encoder skipped both
    // and kept them in columns. Without this, every imported identity reads
    // back as `Unknown` status on mainnet.
    qi.status = IdentityStatus::from(status);
    qi.network = network;

    // The SQL `alias` column is authoritative — the blob's copy is stale.
    // In v0.9.3, `set_identity_alias` wrote ONLY the column, while every
    // identity loader decoded the blob and then unconditionally overwrote
    // `alias` with the column value (`identity.alias = alias;`). A rename or
    // an alias removal therefore left the blob holding the old value, and the
    // column always won at load time. Keeping the blob when populated would
    // resurrect a renamed-away alias or reverse a removal during upgrade, so
    // the column wins here exactly as it did in v0.9.3 — including a NULL
    // column clearing a stale blob alias.
    qi.alias = alias;

    Some(LegacyIdentityRow { id, qi, wallet })
}

/// Decode the six raw columns of one legacy `identity` row. Kept separate so a
/// malformed column storage class is a `Result` the row loop can count and skip,
/// rather than a `?` that escapes [`read_identities`] and discards every row
/// already accumulated. Same contract as [`decode_scheduled_vote_columns`].
///
/// `status` and `wallet_index` are read as signed 64-bit integers on purpose:
/// the legacy schema puts no `CHECK` on either, so a corrupted row can hold a
/// value past the modern `u8` / `u32` range. Widening here and converting in the
/// row loop turns that into a counted, skipped row rather than an
/// `IntegralValueOutOfRange` that a narrow `row.get` would raise through `?`.
#[allow(clippy::type_complexity)]
fn decode_identity_columns(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    Vec<u8>,
    Vec<u8>,
    i64,
    Option<Vec<u8>>,
    Option<i64>,
    Option<String>,
)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

/// Decode the five raw columns of one legacy `scheduled_votes` row. Kept
/// separate so a conversion failure is a `Result` the row loop can count and
/// skip, rather than a `?` that escapes [`read_scheduled_votes`].
fn decode_scheduled_vote_columns(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(Vec<u8>, String, String, u64, i64)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

/// Decode the three raw columns of one legacy `top_up` row. Same contract as
/// [`decode_scheduled_vote_columns`]: a malformed column costs its own row, not
/// the whole read.
fn decode_top_up_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Vec<u8>, u32, u64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

/// Read the top-up history of every identity on `network`.
///
/// The legacy `top_up` table carries no network column, so rows are scoped
/// by joining the legacy `identity` table. An identity whose row is gone
/// contributes nothing — its top-ups are unreachable audit trail.
pub(crate) fn read_top_ups(conn: &Connection, network: Network) -> rusqlite::Result<LegacyTopUps> {
    if !table_exists(conn, "top_up")? || !table_exists(conn, "identity")? {
        return Ok(LegacyTopUps::default());
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

    let mut out = LegacyTopUps::default();
    let mut grouped: BTreeMap<[u8; 32], BTreeMap<u32, u64>> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let (identity_id, index, amount) = match decode_top_up_columns(row) {
            Ok(columns) => columns,
            Err(e) => {
                tracing::warn!(
                    target = "database::legacy_import",
                    error = ?e,
                    "Skipping legacy top-up row with an unreadable column",
                );
                out.unreadable = out.unreadable.saturating_add(1);
                continue;
            }
        };
        let Ok(identity_id) = <[u8; 32]>::try_from(identity_id.as_slice()) else {
            tracing::warn!(
                target = "database::legacy_import",
                blob_len = identity_id.len(),
                "Skipping legacy top-up row with a non-32-byte identity id",
            );
            out.unreadable = out.unreadable.saturating_add(1);
            continue;
        };
        grouped
            .entry(identity_id)
            .or_default()
            .insert(index, amount);
    }

    out.top_ups = grouped.into_iter().collect();
    Ok(out)
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
    use crate::database::test_helpers::{
        LegacyIdentityFixture, basic_legacy_identity_blob, create_legacy_identity_table,
    };
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
        assert!(settings.disable_zmq);
        assert!(!settings.overwrite_dash_conf);
        assert!(!settings.auto_start_spv);
        assert!(!settings.close_dash_qt_on_exit);
        assert_eq!(
            settings.dash_qt_path,
            Some(std::path::PathBuf::from("/opt/dash-qt"))
        );
    }

    /// The legacy `user_mode` column gated nothing, so it records no role. It
    /// must import as "no role chosen" (`None`) — which the app resolves to
    /// [`UserRole::WHEN_UNSET`], the surface the legacy build gave every user —
    /// and never as a concrete role. Seeding `Everyday` off a legacy `Beginner`
    /// would silently strip capability the user already had.
    #[test]
    fn legacy_user_mode_imports_as_no_role_chosen() {
        for legacy_mode in ["Beginner", "Advanced"] {
            let conn = Connection::open_in_memory().unwrap();
            create_settings_table(&conn);
            conn.execute(
                "INSERT INTO settings (id, network, start_root_screen, user_mode, database_version)
                 VALUES (1, 'testnet', 0, ?1, 40)",
                rusqlite::params![legacy_mode],
            )
            .unwrap();

            let settings = read_app_settings(&conn).unwrap().expect("settings row");
            assert_eq!(
                settings.user_role, None,
                "legacy user_mode {legacy_mode} must not seed a role",
            );
        }
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

    /// A present network value is authoritative. If it is unknown, falling
    /// back to the Mainnet default would silently move the user to another
    /// network, so the whole settings import must fail.
    #[test]
    fn app_settings_rejects_an_unknown_network_name() {
        let conn = Connection::open_in_memory().unwrap();
        create_settings_table(&conn);
        conn.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, 'not-a-network', 0, 40)",
            [],
        )
        .unwrap();

        assert!(
            read_app_settings(&conn).is_err(),
            "an unknown saved network must not silently become Mainnet",
        );
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

    /// A malformed column is the same class of damage as an undecodable vote
    /// choice: skip the row, count it, keep going. Aborting the read would
    /// discard every vote already accumulated in the same pass — the valid rows
    /// on both sides of the bad one — and turn a warning into a hard failure.
    /// Both shapes SQLite can hand back are covered: an out-of-range integer
    /// (a negative `time` where a `u64` belongs) and a type mismatch (a blob in
    /// the `vote_choice` text column).
    #[test]
    fn scheduled_votes_malformed_column_skips_only_its_own_row() {
        let conn = Connection::open_in_memory().unwrap();
        create_scheduled_votes_table(&conn, true);
        let voter = [0x11u8; 32];

        insert_vote(&conn, &voter, "before", "Lock", 0, Some("testnet"));
        conn.execute(
            "INSERT INTO scheduled_votes
             (identity_id, contested_name, vote_choice, time, executed, network)
             VALUES (?1, 'negative-time', 'Lock', -1, 0, 'testnet')",
            rusqlite::params![voter.as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scheduled_votes
             (identity_id, contested_name, vote_choice, time, executed, network)
             VALUES (?1, 'blob-choice', ?2, 1700000000, 0, 'testnet')",
            rusqlite::params![voter.as_slice(), vec![0xFFu8; 4]],
        )
        .unwrap();
        insert_vote(&conn, &voter, "after", "Abstain", 0, Some("testnet"));

        let read = read_scheduled_votes(&conn, Network::Testnet).unwrap();

        assert_eq!(read.unreadable, 2, "both malformed rows are reported");
        let names: Vec<&str> = read
            .votes
            .iter()
            .map(|v| v.contested_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["before", "after"],
            "the valid votes on both sides of a malformed row still import",
        );
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

        assert_eq!(top_ups.unreadable, 0);
        assert_eq!(
            top_ups.top_ups.len(),
            1,
            "only the testnet identity's top-ups"
        );
        assert_eq!(top_ups.top_ups[0].0, mine);
        assert_eq!(top_ups.top_ups[0].1, BTreeMap::from([(0, 1000), (1, 2000)]));
    }

    /// A malformed amount (negative, so out of range for `u64`) skips its own
    /// row rather than aborting the read and losing every other identity's
    /// audit trail.
    #[test]
    fn top_ups_malformed_column_skips_only_its_own_row() {
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
        conn.execute(
            "INSERT INTO identity (id, network) VALUES (?1, 'testnet')",
            rusqlite::params![mine.as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO top_up (identity_id, top_up_index, amount) VALUES
                (?1, 0, 1000), (?1, 1, -5), (?1, 2, 2000)",
            rusqlite::params![mine.as_slice()],
        )
        .unwrap();

        let top_ups = read_top_ups(&conn, Network::Testnet).unwrap();

        assert_eq!(top_ups.unreadable, 1, "the malformed row is reported");
        assert_eq!(top_ups.top_ups.len(), 1);
        assert_eq!(
            top_ups.top_ups[0].1,
            BTreeMap::from([(0, 1000), (2, 2000)]),
            "the readable top-ups around a malformed row still import",
        );
    }

    #[test]
    fn top_ups_absent_table_reads_empty() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(
            read_top_ups(&conn, Network::Testnet).unwrap(),
            LegacyTopUps::default()
        );
    }

    // ── Identities ───────────────────────────────────────────────────

    fn create_identity_table(conn: &Connection) {
        create_legacy_identity_table(conn).expect("create identity table");
    }

    /// A genuinely-encodable identity blob, in the legacy `to_bytes()` shape.
    fn identity_blob(id: [u8; 32]) -> Vec<u8> {
        identity_blob_with_alias(id, Some("alias"))
    }

    /// Like [`identity_blob`], but with an explicit alias — `None` yields a blob
    /// whose own alias is absent, which exercises the column fallback.
    fn identity_blob_with_alias(id: [u8; 32], alias: Option<&str>) -> Vec<u8> {
        basic_legacy_identity_blob(id, alias, Network::Testnet)
    }

    /// Stage one local, `Active` legacy identity row on testnet.
    fn insert_identity(conn: &Connection, id: [u8; 32], data: Option<Vec<u8>>) {
        LegacyIdentityFixture::new(id, data, "testnet")
            .insert(conn)
            .expect("insert identity");
    }

    /// The status column is the only source of an identity's status — the
    /// bincode blob does not carry it. Losing it relabels every migrated
    /// identity as "Unknown, refresh required".
    #[test]
    fn identities_restore_status_from_its_column() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let id = [0xAA; 32];
        LegacyIdentityFixture::new(id, Some(identity_blob(id)), "testnet")
            .with_status(IdentityStatus::Active)
            .insert(&conn)
            .expect("insert identity");

        let read = read_identities(&conn, Network::Testnet).unwrap();

        assert_eq!(read.identities.len(), 1);
        assert_eq!(read.identities[0].qi.status, IdentityStatus::Active);
        assert_eq!(read.identities[0].qi.network, Network::Testnet);
        assert_eq!(read.identities[0].id, id);
        assert_eq!(read.unreadable, 0);
    }

    /// v0.9.3 cached observed identities with `is_local = 0` and often a NULL
    /// blob. They are a lookup cache, not the user's own identities — every
    /// v0.9.3 read path filtered them out and so must this one.
    #[test]
    fn identities_skip_observed_and_null_blob_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let mine = [0xAA; 32];
        let observed = [0xBB; 32];
        let null_blob = [0xCC; 32];
        insert_identity(&conn, mine, Some(identity_blob(mine)));
        LegacyIdentityFixture::new(observed, Some(identity_blob(observed)), "testnet")
            .with_is_local(false)
            .insert(&conn)
            .expect("insert identity");
        insert_identity(&conn, null_blob, None);

        let read = read_identities(&conn, Network::Testnet).unwrap();

        assert_eq!(read.identities.len(), 1, "only the local, non-null row");
        assert_eq!(read.identities[0].id, mine);
        assert_eq!(
            read.unreadable, 0,
            "a skipped cache row is not a failure — counting it would withhold the \
             sentinel forever",
        );
    }

    /// The wallet link is what re-attaches an identity to the wallet holding its
    /// keys. It must survive the read exactly as stored.
    #[test]
    fn identities_carry_their_wallet_link() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let id = [0xAA; 32];
        let seed_hash = [0x77; 32];
        LegacyIdentityFixture::new(id, Some(identity_blob(id)), "testnet")
            .with_wallet(seed_hash.to_vec(), 3)
            .insert(&conn)
            .expect("insert identity");

        let read = read_identities(&conn, Network::Testnet).unwrap();
        assert_eq!(read.identities[0].wallet, Some((seed_hash, 3)));
    }

    /// A corrupt blob is counted, never silently dropped, and never blocks the
    /// identities around it.
    #[test]
    fn identities_count_unreadable_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let good = [0xAA; 32];
        let corrupt = [0xBB; 32];
        insert_identity(&conn, good, Some(identity_blob(good)));
        insert_identity(&conn, corrupt, Some(vec![0xFF; 8]));

        let read = read_identities(&conn, Network::Testnet).unwrap();

        assert_eq!(read.identities.len(), 1, "the readable identity survives");
        assert_eq!(read.identities[0].id, good);
        assert_eq!(read.unreadable, 1);
    }

    /// Identities on another network must not leak into this network's import.
    /// A pre-v29 `data.db` still spells mainnet `dash`.
    #[test]
    fn identities_filter_by_network_including_the_legacy_mainnet_spelling() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let testnet_id = [0xAA; 32];
        let legacy_mainnet_id = [0xBB; 32];
        insert_identity(&conn, testnet_id, Some(identity_blob(testnet_id)));
        LegacyIdentityFixture::new(
            legacy_mainnet_id,
            Some(identity_blob(legacy_mainnet_id)),
            LEGACY_MAINNET_ALIAS,
        )
        .insert(&conn)
        .expect("insert identity");

        let testnet = read_identities(&conn, Network::Testnet).unwrap();
        assert_eq!(testnet.identities.len(), 1);
        assert_eq!(testnet.identities[0].id, testnet_id);

        let mainnet = read_identities(&conn, Network::Mainnet).unwrap();
        assert_eq!(
            mainnet.identities.len(),
            1,
            "a pre-v29 mainnet identity must still be found on mainnet",
        );
        assert_eq!(mainnet.identities[0].id, legacy_mainnet_id);
    }

    /// An out-of-range `status` / `wallet_index` is row-level corruption like
    /// any other: counted, skipped, and never allowed to take down the whole
    /// read. SQLite puts no `CHECK` on either column, so a corrupted value
    /// (300 does not fit a `u8`) is storable — and a narrow `row.get` would
    /// raise `IntegralValueOutOfRange` through `?`, losing every readable
    /// identity alongside it, keys included.
    #[test]
    fn identities_skip_out_of_range_column_values_without_failing_the_read() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let good = [0xAA; 32];
        let bad_status = [0xBB; 32];
        let bad_index = [0xCC; 32];

        insert_identity(&conn, good, Some(identity_blob(good)));
        // `status` and `wallet_index` are bound as raw integers, past the u8/u32
        // range the modern types accept.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, network)
             VALUES (?1, ?2, 300, 1, 'testnet')",
            rusqlite::params![bad_status.as_slice(), identity_blob(bad_status)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, wallet, wallet_index, network)
             VALUES (?1, ?2, 2, 1, ?3, 4294967296, 'testnet')",
            rusqlite::params![
                bad_index.as_slice(),
                identity_blob(bad_index),
                [0x77u8; 32].as_slice()
            ],
        )
        .unwrap();

        let read = read_identities(&conn, Network::Testnet)
            .expect("an out-of-range column must not fail the whole read");

        assert_eq!(
            read.identities.len(),
            1,
            "the readable identity must still come across",
        );
        assert_eq!(read.identities[0].id, good);
        assert_eq!(read.unreadable, 2, "both corrupt rows are reported");
    }

    /// A wrong SQLite *storage class* on a column (an INTEGER where the blob is
    /// expected, a BLOB where the `alias` text is expected) must cost only its
    /// own row. Before the per-row decode, these raised `InvalidColumnType`
    /// through `?` and discarded every identity already read this pass — keys
    /// included. The good row, ordered first, proves the batch is not thrown
    /// away when a later row is malformed.
    #[test]
    fn identities_skip_malformed_column_types_without_discarding_read_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);

        let good = [0xAA; 32];
        let bad_data = [0xBBu8; 32];
        let bad_alias = [0xCCu8; 32];

        // Ordered first so a `?`-escape would take it down with the bad rows.
        insert_identity(&conn, good, Some(identity_blob(good)));
        // `data` holds an INTEGER — `row.get::<Vec<u8>>` rejects it. It passes the
        // `data IS NOT NULL` filter, so the read reaches the decode and skips.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, network)
             VALUES (?1, 42, 2, 1, 'testnet')",
            rusqlite::params![bad_data.as_slice()],
        )
        .unwrap();
        // `alias` holds a BLOB — a TEXT-affinity column keeps a blob as a blob, so
        // `row.get::<Option<String>>` rejects it. This is the column added in the
        // prior commit; a malformed one must not fail the whole read either.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, alias, network)
             VALUES (?1, ?2, 2, 1, X'DEADBEEF', 'testnet')",
            rusqlite::params![bad_alias.as_slice(), identity_blob(bad_alias)],
        )
        .unwrap();

        let read = read_identities(&conn, Network::Testnet)
            .expect("a malformed column type must not fail the whole read");

        assert_eq!(
            read.identities.len(),
            1,
            "the readable identity accumulated before the bad rows must survive",
        );
        assert_eq!(read.identities[0].id, good);
        assert_eq!(read.unreadable, 2, "both malformed-type rows are reported");
    }

    /// The SQL `alias` column is authoritative and always wins over the blob's
    /// stale copy. In v0.9.3 `set_identity_alias` wrote only the column, and
    /// every loader decoded the blob then unconditionally overwrote `alias` with
    /// the column value — so a rename or a removal left the blob stale, and the
    /// column always won at load time. The migration must reproduce that: a
    /// populated column overrides a differing blob alias, and a NULL column
    /// clears a stale blob alias (a rename-away / removal the user made in
    /// v0.9.3). Getting this backwards would resurrect a renamed-away alias.
    #[test]
    fn identities_alias_always_takes_the_column() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);

        let column_overrides_blob = [0xAA; 32];
        let column_fills_empty_blob = [0xBB; 32];
        let null_column_clears_blob = [0xCC; 32];

        // Blob carries "blob-alias" but the column holds a newer "column-alias":
        // the column value the user last set must win.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, alias, network)
             VALUES (?1, ?2, 2, 1, 'column-alias', 'testnet')",
            rusqlite::params![
                column_overrides_blob.as_slice(),
                identity_blob_with_alias(column_overrides_blob, Some("blob-alias"))
            ],
        )
        .unwrap();
        // Blob has no alias — the column supplies the value.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, alias, network)
             VALUES (?1, ?2, 2, 1, 'column-alias', 'testnet')",
            rusqlite::params![
                column_fills_empty_blob.as_slice(),
                identity_blob_with_alias(column_fills_empty_blob, None)
            ],
        )
        .unwrap();
        // Blob still holds a stale "blob-alias" but the column is NULL — the user
        // removed the alias in v0.9.3, which wrote only the column. The NULL must
        // win, or the migration resurrects the removed alias.
        conn.execute(
            "INSERT INTO identity (id, data, status, is_local, alias, network)
             VALUES (?1, ?2, 2, 1, NULL, 'testnet')",
            rusqlite::params![
                null_column_clears_blob.as_slice(),
                identity_blob_with_alias(null_column_clears_blob, Some("blob-alias"))
            ],
        )
        .unwrap();

        let read = read_identities(&conn, Network::Testnet).unwrap();

        let alias_of = |id: [u8; 32]| {
            read.identities
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("row not found"))
                .qi
                .alias
                .clone()
        };
        assert_eq!(
            alias_of(column_overrides_blob).as_deref(),
            Some("column-alias"),
            "the column value overrides a differing blob alias",
        );
        assert_eq!(
            alias_of(column_fills_empty_blob).as_deref(),
            Some("column-alias"),
            "the column supplies the alias when the blob has none",
        );
        assert_eq!(
            alias_of(null_column_clears_blob),
            None,
            "a NULL column clears a stale blob alias the user removed in v0.9.3",
        );
        assert_eq!(read.unreadable, 0);
    }

    /// The vault key comes from the id *inside* the blob, but the migration's
    /// skip-if-present precheck keys off the row's `id` column. A row whose two
    /// ids disagree is corruption: importing it would silently overwrite the
    /// unrelated identity the blob names. It is counted and skipped, not imported.
    #[test]
    fn identities_skip_rows_whose_row_id_and_blob_id_disagree() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);

        let good = [0xAA; 32];
        let row_id = [0xBB; 32];
        let blob_id = [0xCC; 32];

        insert_identity(&conn, good, Some(identity_blob(good)));
        // Row `id` column and the blob's embedded id disagree.
        insert_identity(&conn, row_id, Some(identity_blob(blob_id)));

        let read = read_identities(&conn, Network::Testnet).unwrap();

        assert_eq!(read.identities.len(), 1, "only the consistent row imports");
        assert_eq!(read.identities[0].id, good);
        assert_eq!(
            read.unreadable, 1,
            "the divergent row is reported, never imported",
        );
    }

    #[test]
    fn identities_absent_table_reads_empty() {
        let conn = Connection::open_in_memory().unwrap();
        let read = read_identities(&conn, Network::Testnet).unwrap();
        assert!(read.identities.is_empty());
        assert_eq!(read.unreadable, 0);
    }

    // ── Single-identity lookup ───────────────────────────────────────

    /// The lookup returns the requested identity and nothing else, with the
    /// same column-restored status, network and alias the bulk read produces.
    #[test]
    fn identity_row_lookup_returns_only_the_requested_identity() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let wanted = [0xAA; 32];
        let other = [0xBB; 32];
        LegacyIdentityFixture::new(wanted, Some(identity_blob(wanted)), "testnet")
            .with_alias("my-node")
            .with_wallet(vec![0x77; 32], 3)
            .insert(&conn)
            .expect("insert identity");
        insert_identity(&conn, other, Some(identity_blob(other)));

        let found = match read_identity_row(&conn, Network::Testnet, &wanted).unwrap() {
            LegacyIdentityLookup::Found(row) => *row,
            other => panic!("expected Found, got {other:?}"),
        };

        assert_eq!(found.id, wanted);
        assert_eq!(found.qi.alias.as_deref(), Some("my-node"));
        assert_eq!(found.qi.status, IdentityStatus::Active);
        assert_eq!(found.qi.network, Network::Testnet);
        assert_eq!(found.wallet, Some(([0x77; 32], 3)));
    }

    /// An id no row carries is "nothing to recover", not a failure — the
    /// ordinary answer for an identity that was never in the legacy file.
    #[test]
    fn identity_row_lookup_is_absent_for_an_unknown_id() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        insert_identity(&conn, [0xAA; 32], Some(identity_blob([0xAA; 32])));

        assert!(matches!(
            read_identity_row(&conn, Network::Testnet, &[0xEE; 32]).unwrap(),
            LegacyIdentityLookup::Absent,
        ));
    }

    /// A fresh install has no `identity` table at all, and asking about an
    /// identity there must not be an error.
    #[test]
    fn identity_row_lookup_is_absent_when_the_table_is_missing() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            read_identity_row(&conn, Network::Testnet, &[0xAA; 32]).unwrap(),
            LegacyIdentityLookup::Absent,
        ));
    }

    /// The observed-identity cache (`is_local = 0`) and a NULL blob hold no
    /// user keys, so neither is a recovery source. Both must read as absent
    /// rather than unreadable: an unreadable verdict is a typed error the user
    /// would see for an identity that simply has nothing stored.
    #[test]
    fn identity_row_lookup_is_absent_for_cache_and_null_blob_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let observed = [0xBB; 32];
        let null_blob = [0xCC; 32];
        LegacyIdentityFixture::new(observed, Some(identity_blob(observed)), "testnet")
            .with_is_local(false)
            .insert(&conn)
            .expect("insert identity");
        insert_identity(&conn, null_blob, None);

        for id in [observed, null_blob] {
            assert!(
                matches!(
                    read_identity_row(&conn, Network::Testnet, &id).unwrap(),
                    LegacyIdentityLookup::Absent,
                ),
                "{} must read as absent, not unreadable",
                hex::encode(id),
            );
        }
    }

    /// A blob that will not decode is corruption, and must be reported as such
    /// — never as an empty record a merge could treat as "nothing was stored".
    #[test]
    fn identity_row_lookup_reports_an_undecodable_blob() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let corrupt = [0xBB; 32];
        insert_identity(&conn, corrupt, Some(vec![0xFF; 8]));

        assert!(matches!(
            read_identity_row(&conn, Network::Testnet, &corrupt).unwrap(),
            LegacyIdentityLookup::Unreadable,
        ));
    }

    /// A row whose `id` column and embedded blob id disagree names two
    /// different identities; recovering from it would merge one identity's keys
    /// into another. It is corruption here exactly as it is for the bulk read.
    #[test]
    fn identity_row_lookup_reports_a_row_whose_ids_disagree() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let row_id = [0xBB; 32];
        insert_identity(&conn, row_id, Some(identity_blob([0xCC; 32])));

        assert!(matches!(
            read_identity_row(&conn, Network::Testnet, &row_id).unwrap(),
            LegacyIdentityLookup::Unreadable,
        ));
    }

    /// An identity on another network must not be reachable from this one, and
    /// a pre-v29 mainnet row (spelled `dash`) must still be reachable on
    /// mainnet.
    #[test]
    fn identity_row_lookup_scopes_to_the_network_and_its_legacy_spelling() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let legacy_mainnet = [0xBB; 32];
        LegacyIdentityFixture::new(
            legacy_mainnet,
            Some(identity_blob(legacy_mainnet)),
            LEGACY_MAINNET_ALIAS,
        )
        .insert(&conn)
        .expect("insert identity");

        assert!(matches!(
            read_identity_row(&conn, Network::Mainnet, &legacy_mainnet).unwrap(),
            LegacyIdentityLookup::Found(_),
        ));
        assert!(matches!(
            read_identity_row(&conn, Network::Testnet, &legacy_mainnet).unwrap(),
            LegacyIdentityLookup::Absent,
        ));
    }

    /// The two readers share one decoder, so an identity the bulk import would
    /// carry across is byte-for-byte the identity the single-row lookup finds.
    /// A divergence would let recovery merge from a record the migration never
    /// considered importable, or miss one it did.
    #[test]
    fn both_identity_readers_decode_a_row_identically() {
        let conn = Connection::open_in_memory().unwrap();
        create_identity_table(&conn);
        let id = [0xAA; 32];
        LegacyIdentityFixture::new(
            id,
            Some(identity_blob_with_alias(id, Some("blob"))),
            "testnet",
        )
        .with_status(IdentityStatus::NotFound)
        .with_alias("column")
        .with_wallet(vec![0x77; 32], 9)
        .insert(&conn)
        .expect("insert identity");

        let bulk = read_identities(&conn, Network::Testnet).unwrap();
        let single = match read_identity_row(&conn, Network::Testnet, &id).unwrap() {
            LegacyIdentityLookup::Found(row) => *row,
            other => panic!("expected Found, got {other:?}"),
        };

        assert_eq!(bulk.identities.len(), 1);
        assert_eq!(bulk.identities[0].id, single.id);
        assert_eq!(bulk.identities[0].wallet, single.wallet);
        assert_eq!(bulk.identities[0].qi, single.qi);
        assert_eq!(bulk.identities[0].qi.status, single.qi.status);
        assert_eq!(bulk.identities[0].qi.network, single.qi.network);
    }
}
