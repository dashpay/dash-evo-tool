# SPV Module Security Audit Report

**Date**: 2026-02-16
**Auditor**: Security Engineer (automated)
**Scope**: `src/spv/` module (manager.rs, error.rs, mod.rs) and related Cargo.toml dependencies
**Branch**: `refactor/no-separate-spv-thread`
**Commit**: `7f2e70c7`

---

## Executive Summary

The SPV module manages Simplified Payment Verification for the Dash Evo Tool desktop application. The recent refactoring moved the SPV client from a separate OS thread with its own tokio runtime to a spawned task on the main tokio runtime. This audit covers the entire SPV module with emphasis on cryptographic material handling, network security, resource exhaustion, and path traversal risks.

**Overall Risk Assessment**: **Medium**. One high-severity finding related to cryptographic key material not being zeroized after use. Several medium and low findings related to path traversal, unbounded channels, and potential runtime blocking. No critical vulnerabilities found.

---

## Critical Findings

*None identified.*

---

## High Risk Findings

### H-1: Extended Private Key String Not Zeroized After Use

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:699-711`
**Type**: CWE-316 (Cleartext Storage of Sensitive Information in Memory)
**CVSS Estimate**: 6.5 (High - local attacker with memory access)

**Description**: In `load_wallet_from_seed()`, the `seed_bytes` array is correctly zeroized on both the success and error paths (lines 695 and 698). However, the `xprv_str` variable (line 699), which contains the Base58-encoded extended private key, is never zeroized. This string remains in heap memory until the allocator reuses the memory region.

```rust
let xprv = ExtendedPrivKey::new_master(self.network, &seed_bytes).map_err(|e| {
    seed_bytes.zeroize();  // Good: seed zeroized on error
    format!("ExtendedPrivKey::new_master failed: {e}")
})?;
seed_bytes.zeroize();  // Good: seed zeroized on success
let xprv_str = xprv.to_string();  // BAD: String not zeroized after use

let account_options = Self::default_account_creation_options();

let wallet_id = match wm.import_wallet_from_extended_priv_key(&xprv_str, account_options) {
    Ok(id) => id,
    Err(WalletError::WalletExists(id)) => id,
    Err(err) => {
        // xprv_str is dropped here without zeroization
        return Err(format!("import_wallet_from_extended_priv_key failed: {err}"));
    }
};
// xprv_str is dropped later without zeroization
```

Additionally, the `ExtendedPrivKey` struct (`xprv`) itself likely contains the raw private key bytes in memory and is also not zeroized. Whether `ExtendedPrivKey` implements `Zeroize` on `Drop` depends on the upstream `dashcore` crate; this should be verified.

**Impact**: An attacker with access to the process memory (via core dump, swap file, memory forensics, or a separate vulnerability allowing memory reads) could extract the extended private key and derive all wallet keys, gaining full control over the user's funds.

**Remediation**:
```rust
let mut xprv_str = xprv.to_string();
// ... use xprv_str ...
xprv_str.zeroize();
```
Also consider wrapping `xprv` in a `Zeroizing<ExtendedPrivKey>` if the type supports it, or manually zeroing the struct's memory after use.

---

## Medium Risk Findings

### M-1: Path Traversal via `devnet_name` in SPV Data Directory

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:1287-1304`
**Type**: CWE-22 (Path Traversal)

**Description**: The `build_spv_data_dir()` function uses `config.devnet_name` directly as a path component without sanitization:

```rust
Network::Devnet => config
    .devnet_name
    .clone()
    .unwrap_or_else(|| "devnet".to_string()),
```

The `devnet_name` field originates from the `.env` configuration file (parsed via the `envy` crate). If `devnet_name` contains path traversal characters (e.g., `../../etc` or absolute paths), the resulting data directory could point outside the intended SPV data directory.

**Impact**: A malicious or corrupted `.env` file could cause SPV data to be written to or read from an arbitrary directory on the filesystem. Since `.env` is a local file under the user's control, exploitation requires either:
1. A supply-chain attack replacing the `.env` file
2. A separate vulnerability allowing file writes to the config directory
3. Social engineering to trick the user into using a crafted config

The risk is **medium** because `.env` is locally controlled, but the lack of validation is a defense-in-depth failure.

**Remediation**: Validate `devnet_name` to ensure it contains only alphanumeric characters, hyphens, and underscores:
```rust
Network::Devnet => {
    let name = config.devnet_name.clone().unwrap_or_else(|| "devnet".to_string());
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("Invalid devnet_name: contains disallowed characters: {}", name));
    }
    name
}
```

### M-2: Unbounded Channel for SPV Client Commands

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:866`
**Type**: CWE-400 (Uncontrolled Resource Consumption)

**Description**: The `DashSpvClientInterface` command channel uses `tokio::sync::mpsc::unbounded_channel()`. The code comment states "Memory usage is bounded in practice by SPV command processing speed," but this is an assumption, not a guarantee.

```rust
let (command_tx, command_receiver) = tokio::sync::mpsc::unbounded_channel();
```

If the consumer (`monitor_network`) stalls or slows down (e.g., due to network latency, disk I/O pressure, or a malicious peer sending data that triggers slow processing), commands can accumulate without bound.

**Impact**: Potential memory exhaustion denial-of-service. The impact is limited because the `DashSpvClientInterface` is only used for quorum lookups (not high-frequency operations), and the channel is internal (not exposed to external input). However, in theory a slow-processing scenario could cause unbounded growth.

**Remediation**: This is an SDK API constraint (the `DashSpvClientInterface` requires an unbounded channel). The existing comment documents this well. Consider monitoring the channel's length or adding a periodic log warning if the queue grows beyond a threshold. If the SDK API can be modified upstream, switch to a bounded channel.

### M-3: `block_in_place` May Block Main Runtime

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:639-664`
**Type**: CWE-400 (Resource Exhaustion), Performance Degradation

**Description**: The `get_quorum_public_key()` method uses `tokio::task::block_in_place()` with `block_on()` inside it. After the refactoring, all SPV operations now share the main tokio runtime. If a quorum lookup takes a long time (network timeout, unresponsive peer), this blocks a worker thread in the main runtime.

```rust
tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current().block_on(async {
        interface
            .get_quorum_by_height(core_chain_locked_height, llmq_type, qh)
            .await
    })
})
```

With 12 worker threads configured for the main runtime, a few concurrent stalled quorum lookups could starve the UI and other async tasks.

**Impact**: Potential UI freeze or degraded responsiveness if quorum lookups stall. This is not a security vulnerability per se, but could contribute to denial-of-service conditions in edge cases.

**Remediation**: Add a timeout to the quorum lookup:
```rust
tokio::task::block_in_place(|| {
    tokio::runtime::Handle::current().block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            interface.get_quorum_by_height(core_chain_locked_height, llmq_type, qh)
        )
        .await
        .map_err(|_| "Quorum lookup timed out".to_string())?
        // ...
    })
})
```

---

## Low Risk / Informational Findings

### L-1: DNS Resolution of User-Provided Host Without Validation

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:1270-1284`
**Type**: CWE-918 (Server-Side Request Forgery - variant)

**Description**: `primary_peer_socket()` resolves `config.core_host` via `ToSocketAddrs` without any validation of the hostname. The hostname comes from the `.env` configuration file.

```rust
let host = config.core_host.as_str();
let addr = format!("{}:{}", host, port);
addr.to_socket_addrs().ok()?.next()
```

Since this is a desktop application where the user controls the configuration, this is low risk. However, there is no validation that the host is a reasonable value (e.g., not a local service like `localhost:6379` that could be used for SSRF-like attacks if the SPV protocol messages could be crafted to interact with non-SPV services).

**Impact**: Minimal. The SPV protocol handshake would fail against non-SPV services. The user controls the configuration file.

**Remediation**: Consider validating that the port is one of the expected Dash P2P ports, or at minimum log a warning when connecting to unexpected hosts/ports.

### L-2: Error Information Disclosure in Error Messages

**Location**: Various locations throughout `src/spv/manager.rs`
**Type**: CWE-209 (Information Exposure Through Error Messages)

**Description**: Error messages include internal details like lock names, file paths, and internal error strings. Since errors are displayed to the end user via the UI (per the project's error handling convention of `Result<T, String>`), this could expose implementation details.

Examples:
- `"SPV stop_token lock poisoned"` (line 382)
- `"Failed to create SPV data dir: {e}"` which includes the full path (line 289)
- `"client_interface lock poisoned: {e}"` (line 623)

**Impact**: Low. This is a desktop application, so information disclosure is to the local user only. However, if error messages are ever sent to a remote telemetry service, this should be reconsidered.

**Remediation**: Consider using generic user-facing messages while logging detailed errors via `tracing`.

### L-3: No Timeout on Wallet Loading Wait Loop

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:784-815`
**Type**: CWE-835 (Loop with Unreachable Exit Condition) - minor variant

**Description**: The wallet loading wait loop has a 30-second timeout, which is appropriate. However, during this wait, it polls every 50ms while holding and releasing the async read lock repeatedly. This is a minor efficiency concern, not a security issue. The existing implementation is acceptable.

**Impact**: Negligible. The timeout prevents indefinite blocking.

### L-4: Broadcast Channel Lagging Handled Gracefully

**Location**: `/home/ubuntu/git/dash-evo-tool/src/spv/manager.rs:1122-1128, 1158-1164, 1196-1198`

**Description**: All three broadcast channel receivers (sync events, wallet events, network events) properly handle `RecvError::Lagged` by logging a warning and triggering reconciliation where appropriate. This is good defensive programming. No action needed.

### L-5: `cargo audit` Results - Unmaintained Dependencies

**Description**: `cargo audit` reports no known security vulnerabilities in the dependency tree. Two unmaintained crate warnings were found:
- `async-std 1.13.2` (RUSTSEC-2025-0052) - transitive via `dark-light`
- `bincode 1.3.3` and `2.0.1` (RUSTSEC-2025-0141) - transitive via `grovestark` and `platform-version`

These are not direct dependencies of the SPV module and carry no known security impact. They are listed here for completeness.

---

## Dependency Vulnerability Research

### dash-sdk (git dependency, rev d6f4eb9)

**Vulnerabilities Found**: 0 relevant
**Sources Checked**: NVD, GitHub Advisories, web search

No known CVEs were found for the `dash-sdk` Rust crate or the DashPay platform repository at the audited revision. The CVE-2024-21485 result from searches relates to the Plotly Dash Python framework, which is unrelated.

### rusqlite 0.38.0

**Vulnerabilities Found**: 0 relevant to v0.38
**Sources Checked**: CVE Details, NVD, RustSec

Historical vulnerabilities (use-after-free in `update_hook`, `rollback_hook`, etc.) affect versions before 0.26.2 and are not applicable to v0.38.

### tokio 1.46.1

**Vulnerabilities Found**: 0 relevant
**Sources Checked**: CVE Details, RustSec, web search

CVE-2025-62518 (TARmageddon) affects `tokio-tar` / `async-tar`, not the `tokio` runtime itself. No known vulnerabilities in tokio 1.46.1.

### zeroize 1.8.1

**Vulnerabilities Found**: 0 relevant
**Sources Checked**: RustSec, web search

No known CVEs for the zeroize crate.

### aes-gcm 0.10.3 / argon2 0.5.3

**Vulnerabilities Found**: 0 relevant
**Sources Checked**: RustSec, web search

RUSTSEC-2025-0009 affects `ring`'s AES-GCM implementation (panic on 64GB+ data), not the `aes-gcm` crate. Not applicable.

### Similar Solution Research

**Bitcoin SPV light clients** have documented vulnerabilities:
- CVE-2017-12842: Fake SPV proof creation (Bitcoin Core < 0.14). The Dash SPV implementation uses compact block filters (BIP-157/BIP-158) rather than Bloom filters, which mitigates this class of attack.
- Privacy leakage via Bloom filters: Not applicable because the Dash SPV client uses compact block filters (Neutrino-style), which provide better privacy.
- Peer stalling attacks: The `dash-spv` library handles peer management internally via `PeerNetworkManager`. The code properly handles cancellation tokens for clean shutdown, reducing stalling risk.

---

## Positive Security Observations

1. **Proper seed zeroization**: `seed_bytes` is correctly zeroized on both success and error paths in `load_wallet_from_seed()`.
2. **No sensitive data in logs**: A search for tracing calls containing "seed", "priv", "key", "secret", or "password" returned no matches in the SPV module.
3. **Cancellation-aware shutdown**: The SPV loop properly responds to both local `stop_token` and global `global_cancel` cancellation tokens.
4. **Lock poisoning handled gracefully**: All lock operations use helper methods that return `SpvResult` errors instead of panicking on poisoned locks.
5. **Bounded channels for most operations**: The request channel (32), reconcile channel (64), and finality channel (64) all use bounded mpsc channels.
6. **Clean resource cleanup**: The `run_spv_loop` method cleans up storage, interface, network manager, and request channels on exit regardless of the exit reason.
7. **Broadcast lag recovery**: All broadcast channel receivers handle lagged messages gracefully by triggering reconciliation.

---

## Recommendations Summary

| ID | Severity | Finding | Action |
|----|----------|---------|--------|
| H-1 | High | xprv_str not zeroized | Zeroize the string after `import_wallet_from_extended_priv_key` |
| M-1 | Medium | Path traversal via devnet_name | Validate devnet_name characters |
| M-2 | Medium | Unbounded command channel | Document SDK constraint; consider monitoring |
| M-3 | Medium | block_in_place may stall runtime | Add timeout to quorum lookup |
| L-1 | Low | DNS resolution without validation | Consider host/port validation |
| L-2 | Low | Internal details in error messages | Use generic user-facing messages |
| L-3 | Low | Wallet wait loop polling | Acceptable as-is (30s timeout) |
| L-5 | Info | Unmaintained transitive deps | Track upstream updates |

---

## Conclusion

The SPV module demonstrates generally good security practices: proper cancellation handling, graceful lock poisoning recovery, bounded channels for most operations, and no logging of sensitive material. The most significant finding is **H-1** (extended private key string not zeroized), which should be addressed before release. The medium-severity findings (M-1 through M-3) represent defense-in-depth improvements that would strengthen the module's resilience. The refactoring from a separate OS thread to the main tokio runtime does not introduce new security vulnerabilities but does create a tighter coupling that makes M-3 (runtime blocking) more relevant than before.
