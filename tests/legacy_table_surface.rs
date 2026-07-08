//! Legacy `data.db` table surface guard tests (TC-DEV-001/002/003).
//!
//! Normative text (inlined from the finish-unwire test-case spec, a scratch
//! file that was never committed — see
//! `docs/ai-design/2026-05-29-finish-unwire/notes.md` §5 for the retained
//! domain summary):
//!
//! - **TC-DEV-001**: non-test cold-boot code must not contain `FROM wallet`
//!   SELECTs against the legacy `wallet` table, outside the allow-list below.
//! - **TC-DEV-002**: non-test cold-boot code must not read or write the
//!   legacy `utxos` table (`FROM`/`INTO`/`UPDATE`/`DELETE FROM utxos`),
//!   outside the allow-list.
//! - **TC-DEV-003**: non-test cold-boot code must not contain SQL touching
//!   the legacy `single_key_wallet` table, outside the allow-list.
//!
//! In short: the cold-boot read path must never touch the legacy `wallet`,
//! `utxos`, or `single_key_wallet` tables; the only call sites that may
//! still mention those tables are the allow-list below, tethered to the
//! T-DEV-02 commit log (`42b88a15`).
//!
//! The test is intentionally a "spec freeze": if a new file in `src/`
//! starts SELECTing from one of these tables, the test fails and forces
//! either (a) a rewrite to the new path or (b) an explicit allow-list
//! extension here with a tether-reason citation.
//!
//! Brain the size of a planet, and here I am grepping for `FROM wallet`.
//! At least the regex is honest.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const SRC_DIR: &str = "src";

/// Files explicitly permitted to mention the legacy tables, per the
/// T-DEV-02 tether (`42b88a15` commit log) and the revised spec
/// "Allow-listed transitional writes" section.
///
/// Adding to this list requires a tether rationale recorded in the spec.
const ALLOW_LIST: &[&str] = &[
    // Database migration + schema management — runs on cold-boot to
    // create/upgrade tables, including legacy ones for replay safety.
    "src/database/initialization.rs",
    "src/database/mod.rs",
    // Surviving CRUD/helper surface — tethered per `42b88a15`.
    "src/database/wallet.rs",
    "src/database/single_key_wallet.rs",
    "src/database/utxo.rs",
    // T-DEV-02 transitional bridge: import path still writes the legacy
    // `wallet` row so older builds (in-process migration replay) have
    // something to read.
    "src/context/wallet_lifecycle.rs",
    // Migration orchestrator — reads legacy data exactly so it can
    // copy it forward to the new sidecars and then never touch the
    // legacy tables again.
    "src/backend_task/migration/finish_unwire.rs",
    // T-SK-03 protected single-key restore — reads the legacy
    // `single_key_wallet` row's password-encrypted blob to decrypt it
    // with the user's old password and re-import it into the modern
    // vault. A migration read inside the legacy → new boundary, the
    // sibling of finish_unwire.rs; never a cold-boot read.
    "src/backend_task/migration/single_key_restore.rs",
    // Wallet-task entry points handing off to migration orchestrator
    // for retry / state transitions. The READS here are inside the
    // migration boundary (legacy → new), not cold-boot reads.
    "src/backend_task/migration/mod.rs",
    // Test-only helper modules colocated with prod code.
    "src/database/contract.rs",
];

fn allow_list() -> BTreeSet<PathBuf> {
    ALLOW_LIST.iter().map(PathBuf::from).collect()
}

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Returns the line numbers of `pat` in `text`. Deliberately does not try to
/// exempt `#[cfg(test)]` blocks — any false positive that surfaces is caught
/// by the allow-list, and a per-file allow-list entry is a cheaper, more
/// reliable escape hatch than parsing Rust brace structure with string
/// matching.
fn live_matches(text: &str, pat: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pat))
        .map(|(idx, line)| (idx + 1, line.to_string()))
        .collect()
}

fn scan_for_pattern(pat: &str) -> Vec<(PathBuf, usize, String)> {
    let mut files = Vec::new();
    collect_rs_files(Path::new(SRC_DIR), &mut files);
    let allow = allow_list();
    let mut offenders = Vec::new();
    for file in files {
        // Normalize to forward-slash, repo-relative form for allow-list
        // comparison so the test works on Windows too.
        let rel = file
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string();
        let rel_pb = PathBuf::from(&rel);
        if allow.contains(&rel_pb) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for (line_no, line) in live_matches(&text, pat) {
            offenders.push((file.clone(), line_no, line));
        }
    }
    offenders
}

/// TC-DEV-001 — non-test cold-boot code must not contain `FROM wallet`
/// SELECTs outside the documented allow-list.
#[test]
fn tc_dev_001_no_live_readers_of_wallet_table() {
    let offenders = scan_for_pattern("FROM wallet");
    assert!(
        offenders.is_empty(),
        "TC-DEV-001 violation — new live reader of `wallet` table:\n{}\n\nIf this is intentional, add the file to the allow-list in `tests/legacy_table_surface.rs` with a tether-reason citation per the spec.",
        offenders
            .iter()
            .map(|(f, l, line)| format!("  {}:{}: {}", f.display(), l, line.trim()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// TC-DEV-002 — non-test cold-boot code must not write/read `utxos`
/// outside the documented allow-list.
#[test]
fn tc_dev_002_no_live_readers_or_writers_of_utxos_table() {
    let patterns = [
        "FROM utxos",
        "INTO utxos",
        "UPDATE utxos",
        "DELETE FROM utxos",
    ];
    let mut offenders = Vec::new();
    for pat in patterns {
        offenders.extend(scan_for_pattern(pat));
    }
    assert!(
        offenders.is_empty(),
        "TC-DEV-002 violation — new live reader/writer of `utxos` table:\n{}\n\nAdd to allow-list with a tether-reason citation if intentional.",
        offenders
            .iter()
            .map(|(f, l, line)| format!("  {}:{}: {}", f.display(), l, line.trim()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// TC-DEV-003 — non-test cold-boot code must not contain SQL touching
/// the `single_key_wallet` *table* outside the documented allow-list.
///
/// Note: "single_key_wallet" appears widely as a field/module name
/// (`selected_single_key_wallet`, `send_single_key_wallet_payment`,
/// etc.) and those references are unrelated to the legacy SQL surface.
/// The guard targets the SQL keywords specifically.
#[test]
fn tc_dev_003_no_live_readers_or_writers_of_single_key_wallet_table() {
    let patterns = [
        "FROM single_key_wallet",
        "INTO single_key_wallet",
        "UPDATE single_key_wallet",
        "DELETE FROM single_key_wallet",
        "TABLE single_key_wallet",
    ];
    let mut offenders = Vec::new();
    for pat in patterns {
        offenders.extend(scan_for_pattern(pat));
    }
    assert!(
        offenders.is_empty(),
        "TC-DEV-003 violation — new live SQL touching `single_key_wallet` table:\n{}\n\nAdd to allow-list with a tether-reason citation if intentional.",
        offenders
            .iter()
            .map(|(f, l, line)| format!("  {}:{}: {}", f.display(), l, line.trim()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
