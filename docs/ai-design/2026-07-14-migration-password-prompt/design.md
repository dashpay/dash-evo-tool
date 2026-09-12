# Storage Update Password Prompt

## Decision

Older wallet data is copied into the current stores during startup. The event is called a **storage update** in every user-facing surface. The previous SQLite database remains a recovery artifact and is opened read-only.

## UI and backend handshake

The backend copies wallet envelopes and metadata, hydrates the wallets, and registers wallets whose seeds are already available. If protected wallets remain locked, it publishes `MigrationState::AwaitingWalletPasswords` with their seed hashes and waits on `MigrationStatus`'s notification.

The egui frame loop owns the human interaction. It selects one hash, renders a non-dismissible wallet-specific password prompt, and either unlocks the wallet or records a skip. A successful seed promotion or a skip notifies the backend. A failed promotion remains a typed `TaskError`, closes the in-memory wallet again, and does not notify the backend, so the update cannot report success without the seed landing in the current vault.

Complete update runs are serialized on `AppContext`. A shared MCP request therefore joins a desktop run instead of creating a second waiter on the password notification.

Modal state is keyed by wallet seed hash rather than window title. Closing the prompt or switching hashes clears the typed buffer before removing its egui cache entry.

## Interactive capability and headless behavior

`AppContext` starts with `NullSecretPrompt`, whose `SecretPrompt::is_interactive()` capability is false. The desktop boot path installs `EguiSecretPromptHost` before backend construction; standalone MCP and CLI construction do not install it.

When protected wallets require input, the backend checks this explicit host capability before publishing an awaiting state. Without it, the update immediately returns `MigrationError::InteractivePromptUnavailable`, wrapped by the dedicated actionable `TaskError`. No timeout, environment variable, or inferred delay can turn a headless caller into an interactive one.

## Previous database invariant

Desktop and standalone boot open every existing `data.db` with SQLite's `SQLITE_OPEN_READ_ONLY` flag and do not run the historical schema ladder against it. Only an absent, fresh compatibility database may be created and initialized. Every production migration and protected-key reader also opens the source read-only. Migration code contains no drop, delete, or update path for legacy tables. Idempotency uses per-network completion sentinels in `det-app.sqlite`; it never uses table absence.

The current vault may contain both the copied recovery envelope and its current raw or password-protected form. Reads prefer the current form, while the recovery envelope remains available. Tests snapshot `data.db` before a complete two-wallet run, unlock one wallet, skip the other, and require byte-for-byte equality afterward.

## Registration concurrency

Upstream wallet registration is single-flight per wallet seed hash. Concurrent callers share a keyed one-shot outcome cell, so one leader reaches upstream and every follower receives the same success or typed error. Completed flights are removed so a later, non-concurrent user retry can try again. Different wallets can register concurrently.

## Decision 2026-09-11: added a safe non-interactive path

**Decided by:** the project owner (Lukasz Klimek), on 2026-09-11. This deliberately reverses the headless restriction above, for the storage update only.

### Why

Under the rule above, every standalone upgrade of an installation holding a password-protected wallet ends at `StorageUpdateNeedsDesktop`. An operator who runs DET only headless — a server, CI, the migration matrix — had no way to finish that upgrade at all. The rule is a capability decision: never wait for a prompt nobody can render. Nothing on record weighed an explicit, non-interactive password channel against it. Q-HEADLESS in `docs/ai-design/2026-06-02-jit-secret-access/design.md` asked only how a headless caller should *fail*, and the "no environment variable or flag" wording was added later without a recorded rationale.

### What changed

- **MCP tool `app_storage_update`.** It takes one secret parameter, `password: SecretString`, plus the usual optional `network`. The password deserializes straight into guarded, zeroizing memory. Its `Debug` is redacted, and its schema is a plain string with no default or example. It is borrowed down the call chain (`prepare_storage_with_wallet_password` → `run_gated` → `register_migrated_wallets`) for the one call and is never kept in shared state.
- **Drain.** Before the interactive-capability check, a supplied password is tried on every wallet that is still locked. It goes through `AppContext::handle_wallet_unlocked` with `WalletUnlockRetention::UntilStorageUpdateComplete`, the same boundary the desktop prompt submits to.
  - When the password is right, the wallet is registered and keeps its protection: the legacy envelope is re-sealed as Tier-2 under the same password. Its seed is forgotten when the update ends.
  - When the password doesn't open a wallet, the update fails with `MigrationError::WalletPasswordRejected`, surfaced as `TaskError::StorageUpdatePasswordRejected`. Nothing is skipped, and the completion sentinel is withheld.
- **Without a password, nothing changes.** The update still fails at once with `InteractivePromptUnavailable` → `StorageUpdateNeedsDesktop`.
- **det-cli `--password-stdin` / `--password-file <path>`.** These are client-side conveniences only. det-cli reads the password and passes it as the tool's `password` parameter, and the tool never learns where it came from, so the path works the same in-process and over HTTP.
- **Desktop sessions refuse the tool** (`McpToolError::DesktopOwnsPasswordPrompt`). See below.

### Why stdin or a file, and never an environment variable or an argument

| Channel | Exposure | Decision |
|---|---|---|
| Argument (`password=...`) | Every local user can see it through `ps` or `/proc/<pid>/cmdline`, and the shell saves it in history. | Refused by det-cli. |
| Environment variable | Same-user processes can read it through `/proc/<pid>/environ`. Every child process inherits it, and crash reporters, `env` dumps and CI logs capture it. It lives as long as the process does. | Not offered. |
| stdin | The reading process consumes it once. It never appears in the process table or in history. | Accepted, except from a terminal, where it would echo. |
| File | Owner-only files only: refused when any group or other permission bit is set (`mode & 0o077`, the OpenSSH private-key rule). The mode is checked on the opened descriptor, so the file can't be swapped between the check and the read. | Accepted on Unix. Refused elsewhere until an ACL check exists. |

Both accepted sources must hold exactly one line. A single trailing line ending is stripped. Empty, multi-line, over-long (`MAX_PASSPHRASE_LEN`) or non-UTF-8 input is refused. The input is read into one pre-allocated zeroizing buffer, so no reallocation leaves an unwiped copy.

### Why this does not weaken the fail-fast rule

The rule exists so that a headless caller never waits for a person. This path never waits either. The password is supplied before the run starts; a missing one fails at once with the same error as before, and a wrong one fails at once with its own typed error. `NullSecretPrompt` stays non-interactive, and no prompt is added. Signing operations (`SecretPromptUnavailable`) keep the Q-HEADLESS ruling.

### One password, no inferred skip

One password is tried on every locked wallet, so all protected wallets must share it. A wallet it does not open fails the whole update rather than being skipped. Skipping is the user's decision in the desktop prompt, and a mistyped password must not make it for them. Wallets opened before the rejection stay registered and protected. The sentinel is withheld, so a re-run revisits them. The limitation: each headless process starts with every protected wallet locked again, so an installation whose wallets have different passwords must finish its update in the desktop app.

### Desktop sessions refuse

In a desktop session the storage update collects passwords in its own window, and it may be parked on that prompt while holding the preparation gate. The tool therefore refuses instead of opening a second, remote channel into that session, or blocking on the gate until someone answers the window.

### Logging

rmcp logs every inbound request, raw tool arguments included, at DEBUG, and every streamable-HTTP response at TRACE. `logging::sensitive_target_cap()` layers a separate global filter over `RUST_LOG` in every subscriber DET installs. It pins the `rmcp` target at INFO, and because it is ANDed with `RUST_LOG`, no directive can lift it. The same fix closes the equivalent, pre-existing exposure of `core_wallet_import`'s recovery phrase and the masternode keys.

### Residuals

- The JSON-RPC request that carries the password is an ordinary buffer in the transport, which `SecretString` cannot wipe. That buffer is the in-process pipe for standalone det-cli, or the bearer-authenticated HTTP body. Every secret tool parameter shares this residual. det-cli refuses to send a password over HTTP unless the address is loopback (`127.0.0.0/8`, `::1`, `localhost`) or `https` (SEC-004). Other MCP clients must hold themselves to the same rule.
- det-cli reads `--password-stdin` through a duplicated, unbuffered descriptor rather than `std::io::stdin()`, whose process-wide 8 KiB buffer is never wiped (SEC-002). The zeroizing read buffer is the only user-space copy until the transport one above.
- `--password-file` is unavailable on non-Unix platforms until the file's ACL can be checked (TODO in `src/bin/det_cli/password.rs`).

### Known, accepted limitations

Security review 2026-09-11 left two findings open. Both need architectural work beyond this change. `TODO(SEC-00n)` in `src/mcp/tools/meta.rs` marks each one.

- **SEC-001: the desktop check covers only the current process.** `has_interactive_secret_prompt()` recognizes a desktop app embedded in the same process, which is how MCP-over-HTTP reaches a running GUI. A standalone det-cli or `det-cli headless` that shares the data directory with a desktop app in another process is not refused. Both processes can then drive the storage update at the same time. The update's idempotent, sentinel-gated drain bounds the damage, but a proper fix needs a cross-process lock on the data directory.
- **SEC-003: password attempts are not throttled.** Every `app_storage_update` call costs one Argon2id derivation per locked wallet and nothing more. A caller can therefore guess repeatedly, limited only by Argon2id's cost. Reaching the tool already requires the stdio pipe or the HTTP bearer token, so the attacker is local or trusted. Attempt throttling with backoff is the fix.

### Verification

- Unit tests cover the drain (right password, shared password, rejection), the tool (desktop refusal, empty password, redacted `Debug`, plain schema), and det-cli (flags, permissions, the one-line rule, no password in any error).
- The migration matrix boots the real v0.9.3 archive twice. The unchanged no-password scenario must stop at `StorageUpdateNeedsDesktop`. The `password_runs` scenario runs `app-storage-update --password-file` and must migrate both wallets. It logs DET, det-cli and rmcp at `trace`, and fails if any command's output contains the password.
