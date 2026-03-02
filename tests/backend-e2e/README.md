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
| `E2E_WALLET_MNEMONIC` | No | BIP-39 mnemonic for the framework wallet. If unset, a fresh mnemonic is generated and funded via the testnet faucet. Set this to reuse a pre-funded wallet across runs. |
| `DASH_EVO_DATA_DIR` | No (set automatically) | Overridden by the harness to point at a persistent temp directory. Do not set manually. |

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

1. Create a persistent workdir under `/tmp/` keyed by `git rev-parse --short HEAD`.
2. Set `DASH_EVO_DATA_DIR` to the workdir so config and `.env` files land there.
3. Create a SQLite database and `AppContext` for `Network::Testnet`.
4. Start SPV in light-client mode and wait for peer connections (60s timeout).
5. Restore (or generate) the framework wallet from `E2E_WALLET_MNEMONIC`.
6. Register the wallet with `AppContext` (idempotent -- handles "already imported").
7. Wait for SPV to sync the wallet's UTXOs.
8. Top up from the testnet faucet if balance is below 1 DASH.

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
and serves as the funding source for all per-test wallets. If
`E2E_WALLET_MNEMONIC` is set, the same wallet is restored on every run. If not,
a fresh wallet is generated and funded via the testnet faucet.

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
Testnet Faucet  --->  Framework Wallet  --->  Test Wallet A
                                         --->  Test Wallet B
                                         --->  ...
```

### Cleanup

`cleanup::cleanup_test_wallets()` sends remaining funds from all non-framework
wallets back to the framework wallet. It is best-effort (logs errors but does
not panic) because UTXOs may already be spent.

## Framework modules

| Module | Purpose |
|---|---|
| `harness` | Singleton `BackendTestContext` with `OnceCell`, initialization logic, `create_funded_test_wallet` |
| `task_runner` | `run_task()` -- thin wrapper around `AppContext::run_backend_task` with a throwaway channel |
| `wait` | Polling helpers: `wait_for_balance`, `wait_for_spendable_balance`, `wait_for_wallet_in_spv`, `wait_for_spv_peers` |
| `funding` | Testnet faucet HTTP client with retries; `ensure_framework_funded` top-up logic |
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
use crate::harness::ctx;
use crate::task_runner::run_task;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
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
- Always use `#[tokio::test(flavor = "multi_thread", worker_threads = 12)]` to
  match the application's runtime configuration.
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
with `--test-threads=1`. The harness sets environment variables during
initialization (specifically `DASH_EVO_DATA_DIR`), which is only safe in
single-threaded mode.

### Faucet rate limits

The testnet faucet may rate-limit requests. If the faucet fails and the
framework wallet has zero balance, initialization panics. Use
`E2E_WALLET_MNEMONIC` with a pre-funded wallet to avoid faucet dependency.

### Network dependency

These tests require live testnet connectivity. They will fail if:

- The machine has no internet access.
- Dash testnet peers are unreachable.
- Dash Platform (for identity/DPNS tests) is down.

---

<sub>Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
