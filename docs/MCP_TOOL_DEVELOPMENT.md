# Exposing Backend Tasks via MCP/CLI

Reference guide for adding new MCP tools that expose existing `BackendTask` variants to the CLI and MCP clients. Follow this checklist exactly — it enforces the architecture boundaries that keep the GUI, backend, and MCP layers decoupled.

## Architecture Rules

1. **Tools live in `src/mcp/tools/`** — never in the CLI binary (`src/bin/det_cli/`). The CLI discovers tools dynamically; it requires zero code changes when a tool is added.
2. **Tools must not contain business logic.** A tool is a thin adapter: validate parameters, resolve context, dispatch a `BackendTask`, reshape the result. All domain logic stays in `BackendTask` handlers (e.g. `run_identity_task()`, `run_wallet_task()`).
3. **One tool struct = one `BackendTask` dispatch.** If a feature needs multiple backend calls, compose them in the backend layer, not in the tool.
4. **Errors flow through `McpToolError`** — never return raw strings or SDK errors to the client. Use `McpToolError::TaskFailed(e)` for backend errors, `McpToolError::InvalidParam { message }` for input validation.
5. **No GUI dependencies.** Tool code must not reference `egui`, screens, UI components, or `AppAction`. The only bridge to the app is `AppContext` + `BackendTask`.

## Checklist

### 1. Verify the BackendTask exists

Confirm the `BackendTask` variant and its corresponding `BackendTaskSuccessResult` variant exist in `src/backend_task/mod.rs`. If they don't, add them first — that is a separate change with its own review scope.

### 2. Create the tool struct

Add a new file or extend an existing file in `src/mcp/tools/` following the domain grouping (`wallet.rs`, `platform.rs`, `network.rs`, etc.).

```rust
pub struct MyNewTool;
```

### 3. Define parameter and output types

- **Parameters**: Derive `Deserialize` + `schemars::JsonSchema`. Reuse shared types from `src/mcp/tools/mod.rs` (`WalletIdParams`, `NetworkParams`, `EmptyParams`) when they fit. Add `#[serde(default)]` on optional fields.
- **Output**: Derive `Serialize` + `schemars::JsonSchema`. Keep output flat and JSON-friendly — no nested enums, no `Arc`, no internal types. Use primitive types the client can consume directly.
- **Schema quirk**: If a field is `serde_json::Value`, apply `#[schemars(transform)]` to emit `"type": "object"` instead of bare `true` (see `src/mcp/tools/meta.rs`).

### 4. Implement `ToolBase`

```rust
impl ToolBase for MyNewTool {
    type Parameter = MyParams;
    type Output = MyOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> { "domain_object_action".into() }
    fn description() -> Option<Cow<'static, str>> { Some("...".into()) }
    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations::default()
            .read_only(true)      // false if it mutates state
            .destructive(false)   // true if it spends funds, deletes data
            .idempotent(true)     // false if repeated calls have side effects
            .open_world(true))    // true if it talks to the network
    }
}
```

**Naming**: `{domain}_{object}_{action}` — e.g. `core_address_create`, `platform_withdrawals_get`. The CLI converts underscores to hyphens automatically.

**Annotations**: Set these accurately — MCP clients use them to decide confirmation prompts and caching.

### 5. Implement `AsyncTool<DashMcpService>`

Follow the standard invocation sequence:

```rust
impl AsyncTool<DashMcpService> for MyNewTool {
    async fn invoke(
        service: &DashMcpService,
        param: MyParams,
    ) -> Result<MyOutput, McpToolError> {
        // 1. Obtain context
        let ctx = service.ctx().await
            .map_err(|e| McpToolError::Internal(e.to_string()))?;

        // 2. Verify network (skip only for network-info or meta tools)
        resolve::verify_network(&ctx, param.network.as_deref())?;

        // 3. Resolve wallet if needed
        let seed_hash = resolve::wallet(&ctx, &param.wallet_id)?;

        // 4. Wait for SPV sync — ONLY for core-chain tools (see SPV gate rule)
        resolve::ensure_spv_synced(&ctx).await?;
        // For platform-only tools, skip this step and add:
        // // No SPV gate — queries DAPI directly

        // 5. Build and dispatch the backend task
        let task = BackendTask::DomainTask(DomainTask::MyVariant { ... });
        let result = dispatch_task(&ctx, task).await
            .map_err(McpToolError::TaskFailed)?;

        // 6. Match the expected result variant, reshape to output
        match result {
            BackendTaskSuccessResult::MyVariant { .. } => Ok(MyOutput { .. }),
            other => Err(McpToolError::Internal(
                format!("Unexpected task result: {other:?}")
            )),
        }
    }
}
```

**Steps 2–4 are conditional:**
- Skip `verify_network` only for `network_info` and `tool_describe`.
- For destructive tools (`read_only: false`), the `network` parameter **must be required** (not optional with `#[serde(default)]`). Use `resolve::require_network()` instead of `resolve::verify_network()` to prevent accidental cross-network operations that could spend funds on the wrong network.
- Skip wallet resolution if the tool doesn't operate on a wallet.
- **SPV gate rule**: Call `ensure_spv_synced` **only** for tools that operate on the core chain — sending Dash, generating receive addresses, reading core balances, creating asset lock transactions. Skip it for tools that query DAPI / Platform directly (platform address balances, withdrawals, identity credit operations, shielded transfers). When skipping, add a comment: `// No SPV gate — queries DAPI directly`.

### 6. Register in `tool_router()`

In `src/mcp/server.rs`, add one line:

```rust
.with_async_tool::<tools::domain::MyNewTool>()
```

This is the only registration step. The CLI, `list_tools`, and `tool_describe` pick it up automatically.

### 7. Update `docs/MCP.md`

Add the tool to the tool reference table with its name, parameters, and description.

## Don'ts

| Don't | Why |
|-------|-----|
| Put tool logic in `src/bin/det_cli/` | CLI discovers tools dynamically — it must stay tool-agnostic |
| Call `AppContext` methods directly instead of dispatching a `BackendTask` | Breaks the task system contract; backend errors won't be handled uniformly |
| Return `impl Debug` or internal types in output | MCP clients consume JSON — output must be `Serialize` + `JsonSchema` |
| Skip `verify_network` on a stateful tool | Risk of operating on the wrong network |
| Add a `String` field to `McpToolError` for a new category | Add a dedicated enum variant instead |
| Store business logic in the tool | Tools are adapters — logic belongs in backend task handlers |

## File Reference

| File | Role |
|------|------|
| `src/mcp/tools/*.rs` | Tool definitions (ToolBase + AsyncTool) |
| `src/mcp/tools/mod.rs` | Shared parameter types |
| `src/mcp/server.rs` | `tool_router()` registration, `DashMcpService` |
| `src/mcp/dispatch.rs` | `dispatch_task()` — bridges MCP to backend task system |
| `src/mcp/resolve.rs` | `verify_network()`, `wallet()`, `ensure_spv_synced()` |
| `src/mcp/error.rs` | `McpToolError` enum |
| `src/backend_task/mod.rs` | `BackendTask` and `BackendTaskSuccessResult` enums |
| `docs/MCP.md` | End-user tool reference and server configuration |
| `docs/CLI.md` | CLI usage and examples |

## See Also

- [CLAUDE.md](../CLAUDE.md) — project conventions, build commands, architecture overview (MCP section)
- [CONTRIBUTING.md](../CONTRIBUTING.md) — development setup, feature flags, code quality, PR workflow
- [docs/MCP.md](MCP.md) — server modes, configuration, client setup
- [docs/CLI.md](CLI.md) — CLI binary usage and shell completion
