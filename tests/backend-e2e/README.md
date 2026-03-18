# Backend E2E Test Framework

End-to-end tests that exercise Dash Evo Tool backend tasks directly (no GUI)
against a live Dash testnet via SPV. These tests verify that core wallet
operations, identity management, DPNS registration, and Platform queries work
correctly through the same `AppContext` and `BackendTask` pipeline used by the
application at runtime.

## Running the tests

All tests are marked `#[ignore]` to prevent them from running during normal
`cargo test`. They require network access, a funded wallet, and serial
execution.

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
| `--test-threads=1` | Tests share a singleton context and mutate wallet state; parallel execution will fail |

### Environment variables

| Variable | Required | Description |
|---|---|---|
| `E2E_WALLET_MNEMONIC` | Yes | BIP-39 mnemonic for the framework wallet. Must be a pre-funded testnet wallet with at least 10 tDASH. Can be set as a shell env var or in the project root `.env` file (see below). If not set, the test fails with an error message and instructions. |

### `.env` file handling

The harness uses two separate `.env` files for different purposes:

1. **Project root `.env`** -- loaded via `dotenvy::dotenv()` at the start of
   initialization. `dotenvy` merges entries from this file into the process
   environment, so `E2E_WALLET_MNEMONIC` can be defined here instead of (or in
   addition to) a shell export.

   ```bash
   # Example: add to project root .env to persist across sessions
   E2E_WALLET_MNEMONIC="word1 word2 word3 ... word12"
   ```

2. **Workdir `.env`** -- the harness passes a persistent temp directory
   (e.g., `/tmp/dash-evo-e2e-testnet-abc1234/`) as `data_dir` to
   `AppContext::new()`, and calls `ensure_env_file()` to copy the bundled
   `.env.example` into the workdir. `AppContext` reads this file for network
   configuration (testnet Platform endpoints, seeds, etc.).

**Precedence**: a shell-exported `E2E_WALLET_MNEMONIC` takes priority over the
`.env` file value (`dotenvy` does not overwrite existing env vars).

```
Project root .env  →  dotenvy merges into process env
                      (E2E_WALLET_MNEMONIC, required)

Harness passes workdir → /tmp/dash-evo-e2e-testnet-<hash>/
    → ensure_env_file() copies .env.example into workdir
    → AppContext::new(workdir, ...) reads workdir/.env for network config
```

## Architecture

### Shared singleton context

All tests share a single `BackendTestContext` initialized via
`tokio::sync::OnceCell`. The first test to call `ctx().await` triggers
initialization; subsequent calls return the cached reference.

```
ctx().await  -->  OnceCell::get_or_init(BackendTestContext::init)
```

`BackendTestContext` holds:

- **`app_context`** -- a fully initialized `Arc<AppContext>` connected to Dash
  testnet with SPV running
- **`framework_wallet_hash`** -- the `WalletSeedHash` of the "bank" wallet used
  to fund per-test wallets
- **`_workdir`** -- path to a persistent temp directory keyed by git revision
  (e.g., `/tmp/dash-evo-e2e-testnet-abc1234`)

### Initialization sequence

1. Initialize tracing subscriber for structured log output.
2. Create a persistent workdir under `/tmp/` keyed by `git rev-parse --short HEAD`.
3. Copy `.env.example` into the workdir via `ensure_env_file()`.
4. Create a SQLite database and `AppContext` for `Network::Testnet`, passing the workdir as `data_dir`.
5. Start SPV in light-client mode and wait for peer connections (60s timeout).
6. Restore the framework wallet from `E2E_WALLET_MNEMONIC` (required).
7. Register the wallet with `AppContext` (idempotent -- handles "already imported").
8. Wait for SPV to sync the wallet's UTXOs and funds to become spendable (180s timeout).
9. Verify balance is above minimum threshold (10 tDASH).
10. Sweep orphaned test wallets from previous runs back to the framework wallet.

### Persistent workdir

The workdir survives across test runs for the same git revision. This means:

- The SQLite database is reused, so wallets registered in prior runs are already
  present.
- The framework wallet registration handles the "already imported" case
  gracefully.
- SPV sync is faster on repeat runs because prior state may be cached.

Clean the workdir manually if you need a fresh start:

```bash
rm -rf /tmp/dash-evo-e2e-testnet-*
```

## Wallet architecture

### Framework wallet ("the bank")

A single long-lived wallet created during initialization. It holds testnet DASH
and serves as the funding source for all per-test wallets.
`E2E_WALLET_MNEMONIC` must be set to the mnemonic of a pre-funded wallet.

### Test wallets

Each test that needs funds calls `ctx.create_funded_test_wallet(amount_duffs)`.
This method:

1. Generates a fresh random 12-word mnemonic.
2. Creates and registers the wallet with `AppContext`.
3. Waits for SPV to pick up the wallet (30s timeout).
4. Sends `amount_duffs` from the framework wallet via `CoreTask::SendWalletPayment`
   (with retry logic -- up to 5 attempts if "Insufficient funds" due to
   unconfirmed change).
5. Polls until the test wallet balance reaches the expected amount (120s timeout).
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

| Module | What it tests |
|---|---|
| `spv_wallet` | SPV sync, wallet creation and registration, DB persistence |
| `send_funds` | Core payment between two wallets (send and return) |
| `fetch_contract` | Platform contract queries (DashPay, non-existent ID, with descriptions) |
| `identity_create` | Identity registration funded from a wallet |
| `register_dpns` | Full flow: identity creation, DPNS name registration, name search verification |
| `identity_withdraw` | Identity credit withdrawal to a Core address |

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
The SPV WalletManager reports **total balance** (including unconfirmed) but only
includes confirmed/InstantSend-locked UTXOs in its spendable set. This means:

- `total_balance_duffs()` may show funds, but `build_unsigned_payment_tx()` fails
  with "Insufficient funds" because `account.utxos` is empty.
- `confirmed_balance_duffs()` reflects actually spendable funds.

The framework mitigates this with:

- **`wait_for_spendable_balance()`** -- polls `confirmed_balance_duffs()` and
  triggers `reconcile_spv_wallets()` on each iteration.
- **Retry logic in `create_funded_test_wallet()`** -- retries sends up to 5 times
  with 10-second backoff when "Insufficient funds" occurs.
- **Post-send wait** -- after funding a test wallet, waits for the framework
  wallet's change output to become spendable before returning.

Tests that send funds between wallets should use `wait_for_spendable_balance()`
before attempting the send, not just `wait_for_balance()`.

### No InstantSend

The SPV light client processes InstantSend locks but there may be timing gaps
between broadcast and lock receipt. The framework does not explicitly wait for
IS locks -- it polls spendable balance which includes IS-locked UTXOs once
they are processed by SPV.

### Serial execution required

The singleton `BackendTestContext` and shared wallet state mean tests must run
with `--test-threads=1`. The harness shares a single SPV manager and funded
wallet across all tests, which requires serial execution.

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
