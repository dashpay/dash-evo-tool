# PR #604 — Manual Review Guide

Key files for manual review: architectural decisions, reusable patterns, and behavioral changes.
Files that merely apply patterns defined here are excluded (~57 files).

## 1. Core Infrastructure

### [`src/ui/components/message_banner.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35)

Heart of the PR. All other changes depend on patterns defined here.

| What | Lines | Link |
|---|---|---|
| `set_global` idempotency change — no longer resets existing banners | L292–333 | [L292](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R292) |
| `replace_global` docs + empty-text semantics | L338–391 | [L338](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R338) |
| `with_details` now takes `impl Debug` instead of `&str` | L190–210 | [L190](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R190) |
| `ResultBannerExt` — `or_show_error()` on `Result` | L715–740 | [L715](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R715) |
| `OptionBannerExt` — `or_show_error()` + `take_and_clear()` on `Option<BannerHandle>` | L742–770 | [L742](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R742) |
| SEC-003: Eviction log no longer includes message text | L327–330 | [L327](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-5c97c5af2fd0a32afba3e66e5f6a5f7acafb9a73da0d3b89a92f8fc9fca06a35R327) |

**Review focus**: Verify the idempotency change in `set_global` (old behavior reset existing banners, new behavior is a no-op). This is a subtle semantic change that affects all callers.

## 2. Behavioral / Architectural Decisions

### [`src/ui/mod.rs` — ScreenLike trait](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-d64f1e2614b0de74ea77854bc2fb944deaaa5b40a4e37b91a81a94da9bd5ebf8R840)

`display_task_result` default changed from showing "Success" banner to **no-op**. Breaking behavioral change — screens that relied on the default now silently swallow success results.

| What | Lines | Link |
|---|---|---|
| `display_task_result` default → no-op | L840–857 | [L840](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-d64f1e2614b0de74ea77854bc2fb944deaaa5b40a4e37b91a81a94da9bd5ebf8R840) |

**Review focus**: Verify that AppState now handles success banners centrally, and no screen was relying on the old default `display_message("Success", Success)`.

### [`src/app.rs` — Connection banner + network switch](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-8c6f1be9c6b6eb6dc2c76e6a6f2706d76f81aad0ff222c5a9ef4eab78acee7b5)

New `update_connection_banner()` state machine and `clear_all_global` on network switch.

| What | Lines | Link |
|---|---|---|
| `previous_connection_state` + `connection_banner_handle` fields | L93–99 | [L93](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-8c6f1be9c6b6eb6dc2c76e6a6f2706d76f81aad0ff222c5a9ef4eab78acee7b5R93) |
| `clear_all_global` on network switch + INTENTIONAL comment | L875–896 | [L875](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-8c6f1be9c6b6eb6dc2c76e6a6f2706d76f81aad0ff222c5a9ef4eab78acee7b5R875) |
| `update_connection_banner()` — Disconnected/Syncing/Synced FSM | L898–934 | [L898](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-8c6f1be9c6b6eb6dc2c76e6a6f2706d76f81aad0ff222c5a9ef4eab78acee7b5R898) |

**Review focus**: The FSM uses `OverallConnectionState` equality to avoid redundant banner updates. Verify the state transitions cover all edge cases (e.g., rapid Disconnected→Syncing→Disconnected).

### [`src/context/connection_status.rs`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-75d4306c0e7eca30d7e0dbb01b6f6b3d3b3af1e65a22e6de68aa6c92d9b3779f)

Removed error-string-matching logic (`contains("Failed to get best chain lock...")`). Verify this dead code removal is safe — the connection banner in `app.rs` now handles disconnected state via the FSM instead.

| What | Lines | Link |
|---|---|---|
| Removed string-matching error handler | L295–327 | [L295](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-75d4306c0e7eca30d7e0dbb01b6f6b3d3b3af1e65a22e6de68aa6c92d9b3779f) |

## 3. Reusable Pattern Examples (one representative each)

### [`src/ui/tokens/mod.rs` — Shared helpers](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-0e09ce3e1e56a10a8d2cef2e29e7addb8e5a4dff5f78b8e15c50cdc94ef53e06R16)

Defines 3 shared helpers used by ~15 token screens. This is where the DRY pattern originates.

| What | Lines | Link |
|---|---|---|
| `load_identities_with_banner()` | L22–28 | [L22](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-0e09ce3e1e56a10a8d2cef2e29e7addb8e5a4dff5f78b8e15c50cdc94ef53e06R22) |
| `set_error_banner()` | L31–34 | [L31](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-0e09ce3e1e56a10a8d2cef2e29e7addb8e5a4dff5f78b8e15c50cdc94ef53e06R31) |
| `validate_signing_key()` — `&Option<T>` → `Option<&T>` | L40–56 | [L40](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-0e09ce3e1e56a10a8d2cef2e29e7addb8e5a4dff5f78b8e15c50cdc94ef53e06R40) |

### [`src/ui/tokens/claim_tokens_screen.rs` — Constructor error handling](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-e5e6e8b6d4ffd3f1fba60f4bbb7c2e9a6a5db3b2aa3b0acfe7c4bc01fd2a88b5)

Best example of the SEC-001 fix pattern — eliminated `.expect()` panics in constructors, replaced with `or_show_error()` + degraded state.

| What | Lines | Link |
|---|---|---|
| Constructor: single DB load + `or_show_error` + `.or_else` fallback | L77–107 | [L77](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-e5e6e8b6d4ffd3f1fba60f4bbb7c2e9a6a5db3b2aa3b0acfe7c4bc01fd2a88b5R77) |
| Status enum: `WaitingForResult(u64)` → `WaitingForResult` (dropped timestamp) | L45–50 | [L45](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-e5e6e8b6d4ffd3f1fba60f4bbb7c2e9a6a5db3b2aa3b0acfe7c4bc01fd2a88b5R45) |

### [`src/ui/identities/add_existing_identity_screen.rs` — `AddIdentityStatus::Error` variant](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-ce4b3e7e4e4beb1a3c39f4def2f6a86a86a3a3a97e64bbd29b9fa5f1db34c7e7)

Shows the PROJ-005 pattern — status enums no longer carry error strings (moved to global banner). Also shows constructor error handling via `MessageBanner::set_global`.

| What | Lines | Link |
|---|---|---|
| `AddIdentityStatus::Error` (unit variant, no string) | L85–88 | [L85](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-ce4b3e7e4e4beb1a3c39f4def2f6a86a86a3a3a97e64bbd29b9fa5f1db34c7e7R85) |
| Constructor: init error → `set_global` | L126–128 | [L126](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-ce4b3e7e4e4beb1a3c39f4def2f6a86a86a3a3a97e64bbd29b9fa5f1db34c7e7R126) |

### [`src/ui/wallets/send_screen.rs` — BannerHandle lifecycle](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-4b2fd2b6b35fe1d83a5c0e1a80e8a51e03ba4c3f03e5da1bb7498fe43e34e57f)

Most complete example of the `BannerHandle` lifecycle pattern — progress banner with elapsed time, clearing on result, and the `set_send_progress_banner` helper.

| What | Lines | Link |
|---|---|---|
| `send_banner: Option<BannerHandle>` field | L389–392 | [L389](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-4b2fd2b6b35fe1d83a5c0e1a80e8a51e03ba4c3f03e5da1bb7498fe43e34e57fR389) |
| `set_send_progress_banner()` — take_and_clear + set_global + with_elapsed | L650–660 | [L650](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-4b2fd2b6b35fe1d83a5c0e1a80e8a51e03ba4c3f03e5da1bb7498fe43e34e57fR650) |

## 4. Conventions / Docs

### [`CLAUDE.md`](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-82d8aa1d8fd8d9b71c3cf5e7fcade1d8697cc47c6df2e1b0c77ed5b0f01e5e93)

Updated conventions affect all future development.

| What | Lines | Link |
|---|---|---|
| Constructor error handling convention | L58–63 | [L58](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-82d8aa1d8fd8d9b71c3cf5e7fcade1d8697cc47c6df2e1b0c77ed5b0f01e5e93R58) |
| `BannerHandle` lifecycle docs | L173–181 | [L173](https://github.com/dashpay/dash-evo-tool/pull/604/files#diff-82d8aa1d8fd8d9b71c3cf5e7fcade1d8697cc47c6df2e1b0c77ed5b0f01e5e93R173) |

## Summary: 8 files to review, ~57 files skipped

| Priority | File | What to verify |
|---|---|---|
| 🔴 | `message_banner.rs` | `set_global` idempotency change, extension traits |
| 🔴 | `src/ui/mod.rs` | `display_task_result` default → no-op |
| 🔴 | `src/app.rs` | Connection banner FSM, `clear_all_global` on switch |
| 🟡 | `connection_status.rs` | Error-string-matching removal safety |
| 🟡 | `tokens/mod.rs` | Shared helpers (`validate_signing_key` signature) |
| 🟢 | `claim_tokens_screen.rs` | Constructor pattern example |
| 🟢 | `add_existing_identity_screen.rs` | Status enum + init error pattern |
| 🟢 | `send_screen.rs` | BannerHandle lifecycle pattern |
| 📄 | `CLAUDE.md` | Convention accuracy |

The remaining ~57 files are mechanical applications of these patterns — `take_and_clear()`, `or_show_error()`, status enum simplification, and `error_message` field removal. If the patterns above look correct, those files are correct by construction.
