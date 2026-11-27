# Backend E2E Test Framework

End-to-end tests that exercise Dash Evo Tool backend tasks directly (no GUI)
against a live Dash testnet or named devnet via SPV. These tests verify that
Core wallet operations, identity management, DPNS registration, and Platform
queries work through the same `AppContext` and `BackendTask` pipeline used by the
application at runtime.

## Running the tests

Live-network tests are marked `#[ignore]` to prevent them from running during
normal `cargo test`. They require network access and a funded wallet, except
where a test documents otherwise.

The harness routes all SDK-deep work — backend tasks (`run_task`) and direct
`dash_sdk::platform::Fetch` calls in test bodies (`run_on_large_stack`) alike —
through a dedicated runtime with a 32 MB thread stack
(`framework/task_runner.rs`), so deep SDK proof verification no longer requires
`RUST_MIN_STACK` to be set by the caller. Exporting it anyway is harmless
belt-and-suspenders for any code path outside that runtime.

```bash
# Run all backend E2E tests
cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1

# Run a single test
cargo test --test backend-e2e --all-features -- --ignored --nocapture --test-threads=1 test_create_identity
```

**Required flags:**

| Flag | Why |
|---|---|
| `--test backend-e2e` | Selects this test binary (defined by `tests/backend-e2e/main.rs`) |
| `--all-features` | Enables feature-gated dependencies |
| `--ignored` | Tests are `#[ignore]` by default |
| `--nocapture` | Shows progress output (SPV sync, balance polling) |
| `--test-threads=1` | Required: all tests share one singleton `BackendTestContext`/SQLite DB; running on parallel OS threads races on DB access (e.g. `AlreadyOpen` errors) |

### Environment variables

| Variable | Required | Description |
|---|---|---|
| `E2E_WALLET_MNEMONIC` | Yes | BIP-39 mnemonic for the framework wallet on the selected network, funded with at least 10 DASH on that network. Set it in the shell or in the project root `.env`. |
| `E2E_NETWORK` | No | `testnet` (default) or `devnet`. Devnet needs `E2E_ENV_FILE` with `DEVNET_dapi_addresses`, `DEVNET_core_host` (SPV peer, Core P2P port 20001) and `DEVNET_devnet_name`; see [Running against a devnet](#running-against-a-devnet). |
| `E2E_DEVNET_BOOTSTRAP` | No | Set to `1` with `E2E_NETWORK=devnet` to let `devnet_setup::fund_framework_platform_address` move funds on Devnet. |
| `E2E_PLATFORM_FUND_DUFFS` | No | Amount bootstrap moves from Core to the Platform address (default 2,500,000,000 duffs or 25 DASH). Set a positive whole number of duffs within the Core money range. |
| `E2E_ENV_FILE` | For devnet | Path to the network configuration to copy into each fresh workdir. With no path set, Testnet uses the bundled `.env.example` for fresh workdirs. |

### Running against a devnet

Testnet is the default. To run against a devnet:

1. Write a `.env` with the devnet's `DEVNET_dapi_addresses`, `DEVNET_core_host`
   (a Core node whose P2P port 20001 is reachable; SPV syncs from it) and
   `DEVNET_devnet_name`, and point `E2E_ENV_FILE` at it.
2. Set `E2E_NETWORK=devnet` and fund the framework wallet from the devnet
   (the init failure prints its address).
3. Run `devnet_setup::fund_framework_platform_address` once with
   `E2E_DEVNET_BOOTSTRAP=1`, so the DPNS tests that fund identities from
   Platform addresses find credits. Budget the transfer amount
   (`E2E_PLATFORM_FUND_DUFFS`, 25 DASH by default), the Core transaction fee,
   and at least 10 DASH to leave in the framework wallet's Core balance for
   the harness. After funding, unset `E2E_DEVNET_BOOTSTRAP` in your shell,
   and remove it from the project `.env` (or set it to `0`) if you put it
   there: the tests load that file, so the flag would stay on.

If bootstrap reports an error, check the framework wallet's Platform address
balance before rerunning it. The transfer may have succeeded before the balance
refresh failed, and rerunning bootstrap could send the full amount again.

Each devnet uses `<tmp>/dash-evo-e2e-devnet-<name-hash>`, with a numeric suffix
for fallback slots. A saved `.env` must match `E2E_ENV_FILE` byte for byte.
If the harness reports a mismatch or missing configuration in a used slot,
move that workdir aside or retry with the original configuration. Also move
the workdir aside when the devnet deployment changes between runs (for
example, new endpoints or a reset chain), because the harness reuses its saved
state as long as `E2E_ENV_FILE` is unchanged. If variable
substitution changes the resolved devnet settings within a test process, use
stable values in `E2E_ENV_FILE` and restart the tests.
Devnet SPV sync needs DET to send the devnet name and user agent to dash-spv,
which it does from #1250 (in `v1.0-dev`), and `DEVNET_devnet_name` set in the
env file: without the name, dash-spv refuses to start on Devnet.

### What the DPNS registration tests cost

The tests in `register_dpns` that fund an identity from platform addresses, and
every test in `measure_dpns_registration`, spend platform address credits on
each run: about 190 million to create the identity, 75 to 80 million for the
registration, and about 13.5 million that cannot be sent back from the identity
afterwards. The rest returns to the funding address.
`measure_dpns_registration_balance_the_network_requires` pays for a registration
step that is refused on purpose.

`test_register_contested_dpns_name_funded_from_platform_addresses` and
`measure_dpns_registration_contested` request a name that needs a community
vote. Each also spends the community vote fee (0.1 or 0.2 tDASH, by protocol
version), which the network never returns, and opens a public vote on testnet.

### Masternode tests

`identity_masternode_withdraw` needs a masternode or evonode on the selected
network with funded Platform credits and its private keys. These are the only
names the tests read:

| Variable | Needed by | Description |
|---|---|---|
| `E2E_MN_PROTX_HASH` | every case except TC-MN-007 and TC-MN-021 | ProTxHash of the node (hex). |
| `E2E_MN_OWNER_KEY` | TC-MN-017, -018, -050, -052 | Owner private key (WIF or 64-hex). |
| `E2E_MN_PAYOUT_KEY` | TC-MN-016, -018, -019, -023, -051, -053, -054 | Payout (transfer) private key (WIF or 64-hex). |
| `E2E_MN_VOTING_KEY` | TC-MN-019 | Voting private key (WIF or 64-hex). |
| `E2E_MN_NODE_TYPE` | all (optional) | `evonode` (default) or `masternode`. |

A case whose variable is unset or blank **fails** with instructions naming the
variable; it never reports `ok` without running. To run the rest of the suite on
a machine without a masternode, skip the module explicitly:

```bash
cargo test --test backend-e2e --all-features -- --ignored --test-threads=1 \
  --skip identity_masternode_withdraw::
```

The earlier names `E2E_MN_PRO_TX_HASH`, `E2E_MN_OWNER_WIF`, `E2E_MN_PAYOUT_WIF`
and `E2E_MN_VOTING_WIF` are not read. If one is set, the failure message names its
replacement. They are not accepted as aliases because two spellings of the same
secret leave precedence ambiguous when both are set.

The withdrawal cases move a tenth of the node's balance, clamped to the
protocol's per-withdrawal limits (`system_limits.min_withdrawal_amount` and
`max_withdrawal_amount`). A node below the minimum fails with a request to fund
it.

### `.env` file handling

The harness uses two separate `.env` files for different purposes:

1. **Project root `.env`** is loaded before the harness resolves the network,
   funding options, and mnemonic. Set the `E2E_*` options here or in the shell.
   Shell values take precedence.

   ```bash
   # Example: add to project root .env to persist across sessions
   E2E_WALLET_MNEMONIC="word1 word2 word3 ... word12"
   ```

2. **Workdir `.env`** provides the network settings to `AppContext`. The
   harness prepares it before opening stores. When `E2E_ENV_FILE` is set,
   the harness copies it into a private workdir and creates the file with
   mode `0600` on Unix. With no override, the harness seeds Testnet workdirs
   from the bundled `.env.example`.

## Architecture

### Shared singleton context

Most live-network tests share a single `BackendTestContext` initialized via
`tokio::sync::OnceCell`. The first test to call `ctx().await` triggers
initialization; subsequent calls return the cached reference.

```
ctx().await  -->  OnceCell::get_or_init(BackendTestContext::init)
```

`BackendTestContext` holds:

- **`app_context`** holds the selected network and workdir, with SPV running.
- **`framework_wallet_hash`** -- the `WalletSeedHash` of the "bank" wallet used
  to fund per-test wallets

### Initialization sequence

1. Initialize tracing and resolve the test settings and framework wallet seed.
2. Lock a persistent workdir for the selected network and prepare its `.env`.
3. Open the stores and create the SQLite database and `AppContext` in that workdir.
4. Register the framework wallet, reusing it if it already exists.
5. Start SPV in light-client mode and wait for peer connections and full sync.
6. Verify the framework wallet has at least 10 DASH and sweep orphaned test
   wallets from previous runs back to it.

The full SPV sync wait is progress-aware and bounded in `framework/wait.rs`.

| Constant | Value | Meaning |
|---|---|---|
| `SPV_STALL_WINDOW` | 180s | Fails the init if the SPV progress token does not advance for this long. |
| `SPV_SYNC_CAP` | 1800s | Hard cap on the whole sync wait, even while it progresses. |

A slow but advancing sync keeps waiting on the same runtime and workdir slot,
so its stored headers and filters are reused. Failing into an init retry would
move to a fresh slot (the previous slot's SPV lock cannot be released
in-process) and restart the sync from genesis. A fresh workdir needs a full
sync, which can take well over 10 minutes. Later runs resume from the stored data.

### Persistent workdir

The base workdir is `<tmp>/dash-evo-e2e-testnet` for Testnet and includes the
devnet name hash for Devnet. If a slot is held by another process,
`open_available_workdir` tries numbered fallback slots using a per-slot
`.lock` file.

- The SQLite database is reused across runs, so wallets registered previously
  are already present, regardless of which commit produced them.
- The framework wallet registration handles the "already imported" case
  gracefully.
- SPV sync is faster on repeat runs because prior state may be cached.

For a fresh start, stop any running backend E2E tests and remove their local
workdirs. This also removes copied `.env` files, including any credentials in
the legacy shared `dash-evo-e2e-devnet` workdir and any leftover reconnect or
cold-boot workdirs. If the tests used a temporary directory other than `/tmp`,
set `TMPDIR` to that directory for this command.

```bash
rm -rf -- /tmp/dash-evo-e2e-devnet \
  "${TMPDIR:-/tmp}"/dash-evo-e2e-{testnet,devnet,reconnect-,coldboot-}*
```

## Wallet architecture

### Framework wallet ("the bank")

A single long-lived wallet created during initialization. It holds DASH on the
selected network and serves as the funding source for all per-test wallets.
`E2E_WALLET_MNEMONIC` must be set to the mnemonic of a pre-funded wallet.

### Test wallets

Each test that needs funds calls `ctx.create_funded_test_wallet(amount_duffs)`.
This method:

1. Generates a fresh random 12-word mnemonic.
2. Creates and registers the wallet with `AppContext`.
3. Waits for SPV to pick up the wallet (30s timeout).
4. Sends `amount_duffs` from the framework wallet via `CoreTask::SendWalletPayment`.
5. Waits for the full `amount_duffs` to become spendable in the test wallet (120s timeout).
6. Waits for the framework wallet's change output to become spendable (so the
   next call to `create_funded_test_wallet` can succeed).

The returned `(WalletSeedHash, Arc<RwLock<Wallet>>)` tuple is ready for use in
backend tasks.

### Funding flow

```
Pre-funded Wallet  --->  Framework Wallet  --->  Test Wallet A
(E2E_WALLET_MNEMONIC)                      --->  Test Wallet B
                                           --->  ...
```

### Cleanup

`cleanup::cleanup_test_wallets()` sends remaining funds from all non-framework
wallets back to the framework wallet. It is best-effort (logs errors but does
not panic) because UTXOs may already be spent.

Cleanup runs automatically during initialization -- the harness sweeps orphaned
test wallets from previous runs (e.g., if a test panicked before cleanup).
Wallets persist in the DB across runs, so AppContext loads them automatically
and SPV syncs their balances.

## Framework modules

Located in `tests/backend-e2e/framework/`:

| Module | Purpose |
|---|---|
| `harness` | Singleton `BackendTestContext` with `OnceCell`, initialization logic, `create_funded_test_wallet` |
| `task_runner` | `run_task()` -- thin wrapper around `AppContext::run_backend_task` with a throwaway channel |
| `wait` | Polling helpers: `wait_for_balance`, `wait_for_spendable_balance`, `wait_for_wallet_in_spv`, `wait_for_spv_peers` |
| `funding` | Balance verification; testnet faucet HTTP client (available as helper, not used in main flow) |
| `identity_helpers` | `build_identity_registration` (key derivation), `get_receive_address` |
| `cleanup` | Best-effort return of test wallet funds to the framework wallet |

## Test modules

Registered in `tests/backend-e2e/main.rs` -- that file is the authoritative
list; keep this table in sync when adding or removing a module.

| Module | What it tests |
|---|---|
| `cleanup_only` | Standalone cleanup -- sweeps orphaned test wallets on init |
| `devnet_setup` | Devnet bootstrap: funds the framework wallet's platform address |
| `fetch_contract` | Platform contract queries (DashPay, non-existent ID, with descriptions) |
| `identity_create` | Identity registration funded from a wallet |
| `identity_masternode_withdraw` | Headless masternode/evonode load + credit withdrawal |
| `identity_withdraw` | Identity credit withdrawal to a Core address |
| `platform_info` | Read-only `PlatformInfo` queries: live current-epoch fetch, completed/queued withdrawal queries |
| `measure_dpns_registration` | Measurements, not checks: what each DPNS registration document costs and what balance the network wants, logged on `MEASURE` lines |
| `register_dpns` | Full flow: identity creation, DPNS name registration, name search verification; also from platform address credits, which are returned afterwards except for about 13.5 million credits per identity that the network does not let a transfer take |
| `send_funds` | Core payment between two wallets (send and return) |
| `spv_wallet` | SPV sync, wallet creation and registration, DB persistence |
| `tx_is_ours` | `is_ours` flag correctness for SPV transactions |
| `identity_cold_boot` | Identity funding on a cold-booted watch-only wallet (scenarios C/D) |
| `spv_reconnect` | SPV reconnect regression (`stop_spv` + `ensure_wallet_backend_and_start_spv`) |
| `core_tasks` | `CoreTask` variants (TC-001 to TC-012) |
| `cross_wallet_topup` | Cross-wallet identity top-up, including spent-lock reuse rejection (#954/#956) |
| `event_bridge_live` | Live `EventBridge` wiring: SPV sync -> `ConnectionStatus` -> frame-loop repaint |
| `identity_in_vault_sign` | TS-SIGN-E2E-01 -- state transition signed by a migrated `InVault` identity key |
| `identity_tasks` | `IdentityTask` variants (TC-020 to TC-030) |
| `shielded_tasks` | `ShieldedTask` variants (TC-074 to TC-083); skippable via `E2E_SKIP_SHIELDED` |
| `token_tasks` | Full token lifecycle: registration, query, mint, burn (TC-045 to TC-065) |
| `wallet_reregistration` | Wallets re-register with upstream SPV so received funds stay visible |
| `wallet_tasks` | `WalletTask` variants (TC-012 to TC-019) |
| `z_broadcast_st_tasks` | `BroadcastStateTransition` (TC-066, TC-067) |

## Writing new tests

### Step 1: Create a test module

Add a new file in `tests/backend-e2e/` and register it in `main.rs`:

```rust
// tests/backend-e2e/main.rs
mod my_new_test;
```

### Step 2: Write the test function

Follow this pattern:

```rust
use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_my_feature() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // If your test needs a funded wallet:
    let (_seed_hash, wallet_arc) = ctx.create_funded_test_wallet(1_000_000).await;

    // Construct and run a backend task:
    let task = BackendTask::SomeDomainTask(/* ... */);
    let result = run_task(app_context, task)
        .await
        .expect("Task should succeed");

    // Assert on the result variant:
    match result {
        BackendTaskSuccessResult::ExpectedVariant(data) => {
            assert!(/* ... */);
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}
```

Key points:

- Always use `#[ignore]` so the test does not run in CI unit test jobs.
- Always use `#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]`.
  The `shared` runtime is critical -- SPV spawns background tasks via `tokio::spawn`
  that are bound to the runtime that created them. With `#[tokio::test]`, each test
  creates its own runtime; when the first test exits, its runtime drops and kills
  the SPV tasks, causing "channel closed" errors in later tests.
- Call `ctx().await` as the first line -- it initializes SPV and the framework
  wallet on first use.
- Use `run_task()` to execute backend tasks. It creates a throwaway MPSC channel
  and returns the `Result<BackendTaskSuccessResult, TaskError>` directly.
- Match on the specific `BackendTaskSuccessResult` variant you expect. Panic on
  unexpected variants to get clear failure messages.

### Step 3: Register the module

Add `mod my_new_test;` to `main.rs`. The test binary is defined by the
`tests/backend-e2e/main.rs` entry point -- no Cargo.toml changes needed.

## Known limitations

### SPV UTXO spendability timing

After broadcasting a transaction, the change output is not immediately spendable.
The upstream `platform-wallet` engine drives chain sync and pushes wallet state
through the `EventBridge` into a per-wallet display snapshot. The snapshot's
`total` includes unconfirmed funds, while `confirmed` reflects the
confirmed/InstantSend-locked set actually usable for spending. This means:

- `snapshot_balance().total` may show funds, but a send fails with
  "Insufficient funds" while the change output is still unconfirmed.
- `snapshot_balance().confirmed` reflects actually spendable funds.

The framework mitigates this with:

- **`wait_for_spendable_balance()`** -- polls
  `AppContext::snapshot_balance().confirmed` (the EventBridge-pushed snapshot),
  waiting until confirmed funds reach the target.
- **Post-send wait** -- after funding a test wallet, `create_funded_test_wallet()`
  waits for the full funded amount to become spendable, then waits for the
  framework wallet's change output to settle before returning.

Tests that send funds between wallets should use `wait_for_spendable_balance()`
before attempting the send, not just `wait_for_balance()`.

### No InstantSend

The SPV light client processes InstantSend locks but there may be timing gaps
between broadcast and lock receipt. The framework does not explicitly wait for
IS locks -- it polls spendable balance which includes IS-locked UTXOs once
they are processed by SPV.

### Faucet availability

The testnet faucet helper (`funding::request_faucet_funds`) is available but not
called during normal initialization. The framework wallet must be pre-funded.
Use the faucet helper manually if needed.

### Network dependency

These tests require live testnet connectivity. They will fail if:

- The machine has no internet access.
- Dash testnet peers are unreachable.
- Dash Platform (for identity/DPNS tests) is down.

---

<sub>Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
