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
