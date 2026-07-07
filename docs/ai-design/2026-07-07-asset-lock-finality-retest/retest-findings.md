# Asset-Lock FinalityTimeout — Retest Findings (post platform-bump)

**Date:** 2026-07-07
**Branch:** `feat/identity-onboarding-ux` @ `866bbf5f` (merge of `cf9c16dd` — platform pin bump
`e6b508f1` → `c2135800` / rust-dashcore `5a1fdf2b` → `647fa982`, 108 upstream commits)
**Author:** Marvin (QA — verification only, no fix applied)
**Relates to:** `docs/ai-design/2026-07-01-asset-lock-finality-rootcause/investigation-findings.md`
(original investigation, PR 860 @ `637dc603`) — **not modified by this doc.**

**Verdict: STILL BROKEN.** Same end-user symptom reproduces on current HEAD. The upstream
fix that *did* land does not cover the failure mode observed here.

---

## 1. What shipped since the original investigation

`git merge-base --is-ancestor 417d61da <current rust-dashcore pin>` → **yes**. Commit
`417d61da fix(key-wallet): don't build asset locks on unconfirmed funds (#836)` — the fix
tracked in MemCan (superseding an earlier "no funding-input finality gate" theory) for the
0-conf-UTXO coin-selection defect — **is included** in the current pin (`647fa982`). This
directly addresses one candidate mechanism from the original H1. It did not fix the bug.

## 2. Repro mechanics and a methodology note (own error)

Built cleanly (warm from `cf9c16dd`'s own gate, `cargo build --test backend-e2e
--all-features` finished in 5.87s, no errors/warnings).

First attempt used the harness's default `RUST_LOG` filter (`backend_e2e=info` — see
`tests/backend-e2e/framework/harness.rs:220-221`), which **filters out all
platform_wallet/key_wallet/dash_spv logs** (`wait_for_proof`, `FinalityTimeout`,
`context=mempool`, ChainLock/InstantSend events — none of it surfaces without an explicit
`RUST_LOG`). Killed that run (~4 min in, still short of the 300s wait) rather than waste a
full run on an uninstrumented capture — **this was my own error**: killing a process
mid-broadcast raced the framework wallet's local UTXO-reservation bookkeeping against the
SIGKILL, and the next attempt (using the same shared, non-git-hash-keyed workdir —
`/tmp/dash-evo-e2e-testnet`, contradicting the harness README's documented "keyed by git
revision" behavior, itself a minor doc/behavior mismatch worth a follow-up) came up with
`spendable: 0, total: 0` and panicked in harness init (`harness.rs:477`). A third attempt
(let run to completion, un-killed) resynced fine (balance `15,082,365,339` duffs, correctly
down ~100,297 duffs from run 1's already-broadcast tx) and produced the clean, fully
instrumented repro analyzed below. No workdir/DB was wiped; all three log files retained.

Logs (full, retained as evidence — quote only excerpts below):
- Run 2 (harness-init panic, collateral of my kill): `/data/tmp/backend-e2e-tc004-96Ggen.log`
- Run 3 (clean instrumented repro, **this is the evidence run**): `/data/tmp/backend-e2e-tc004-tqNtw6.log`

## 3. Run 3 result

```
thread 'core_tasks::test_tc004_create_registration_asset_lock' (2524905) panicked at tests/backend-e2e/core_tasks.rs:140:10:
CreateRegistrationAssetLock should succeed: WalletBackend { source: FinalityTimeout(OutPoint { txid: 0x27970abaa46660e45a9e2c5c62328aa16d4ebab2e2c396edec14d417545ef013, vout: 0 }) }
test result: FAILED. 0 passed; 1 failed; ... finished in 308.04s
```

Identical symptom shape to the original doc: `WalletBackend { source: FinalityTimeout(OutPoint {..}) }`
after the 300s `wait_for_proof` window (`platform_wallet::wallet::asset_lock::sync::proof::wait_for_proof`).

## 4. Hard on-chain evidence (Insight testnet) — root mechanism identified precisely

Our broadcast asset-lock tx `27970abaa4…` and its reversed-byte form: **Not found** on
Insight (both orders) — never reached the real network, consistent with the original doc.

But this time the *funding input* resolves cleanly, and that's the finding:

| Tx | Insight result |
|---|---|
| `27970abaa46660e45a9e2c5c62328aa16d4ebab2e2c396edec14d417545ef013` (our new asset-lock tx) | **Not found** |
| `a16e8db4aa2676f47a8b1f93ebd6ba8b16fcda22411962ed6d603ba6277858f7:1` (the input our tx spends, value 19.99796384 DASH) | **Confirmed**, block 1479018, 31046 confirmations. `vout[1]` carries `"spentTxId":"1c6e3f9dc640ebda4d6d97d15743cc3fc0cdcb8cf6253c7b440499c57c6707a4","spentHeight":1479019` |
| `1c6e3f9dc640ebda4d6d97d15743cc3fc0cdcb8cf6253c7b440499c57c6707a4` (the *real* spender of that input, one block later) | Confirmed, block 1479019, 31045 confirmations |
| `addr/ygomtzTZPGtJ3e6xVTr7sqMDK4oLTwUB2o` (the input's address — owned by the framework wallet; DET's own coin-selection chose it) | `balance: 0`, `totalReceived == totalSent == 19.99796384`, `txApperances: 2`, `unconfirmedTxApperances: 0` |

**The framework wallet's own local UTXO index still believes an output that Insight shows
was spent ~31,044 blocks (≈54 days at ~2.5 min/block) ago is unspent and spendable.** DET's
coin-selection picked exactly this stale phantom UTXO as the sole funding input for a brand
new asset-lock transaction. Any peer running real chain-state validation rejects that as a
double-spend (`bad-txns-inputs-missingorspent` — same rejection class independently
captured and quoted in the prior session's MemCan record for an earlier phantom chain).
Broadcasting it can never produce an IS-lock or ChainLock, so `wait_for_proof` is
**guaranteed** to exhaust its 300s budget regardless of network health.

Corroboration that the SPV client's finality machinery itself is *not* the problem this
time (a real change from the original doc, where InstantSend showed 0 activity for 18+
minutes): a **different**, unrelated transaction (`1435e9423af1b2be7e6f2408ab59dcc97d8c55fea080d8aa47eea89dea9db989`)
received a real IS-lock during our exact wait window (`txlock: true`, confirmed next block),
and our wallet's ChainLock height advanced three times during the 300s wait
(`1510061 → 1510062 → 1510063`, each independently `Synced`/`valid` in the log) while our
tx's own transaction-record context stayed stuck at `"Mempool"` for all four `wait_for_proof`
iterations — because it was never actually accepted anywhere to begin with.

## 5. Which hypothesis does this confirm/refute?

- **H1 (local spendability judged incorrectly, not from network finality)** — **confirmed,
  in a sharper form.** The original doc's H1 was about 0-conf/mempool change being
  misclassified `Confirmed`. Here the wallet's *persisted* index carries a UTXO that is
  fully, unambiguously spent on real chain history from ~54 days ago — not a
  confirmation-timing race, a **stale/never-invalidated UTXO-set entry**. The rust-dashcore
  #836 fix (§1) only guards against spending *unconfirmed* funds; it has no mechanism to
  detect "confirmed-according-to-local-index, but actually long-spent-on-chain," so it does
  not touch this failure mode at all.
- **H2 (txs never propagate to network mempool)** — not the primary mechanism this time;
  our tx never propagating is a *consequence* of H1 (peers correctly reject a double-spend),
  not evidence the network/relay path itself is broken.
- **H3 (InstantSend not functioning)** — **refuted.** IS-locks are being received and
  processed normally on this run (see §4); the earlier "0 valid IS-locks in 18 minutes"
  observation does not reproduce.

## 6. Scope note — is this a shared-workdir artifact or a real product defect?

The specific stale UTXO discovered here lives in a **long-lived, never-wiped, non-git-hash-keyed
E2E workdir** (`/tmp/dash-evo-e2e-testnet`) accumulated across many past sessions over ~54
days. It's plausible this exact address's history predates a wallet rescan checkpoint and
was never re-validated. That does not make it a non-finding: the underlying defect it
exposes — **DET/platform-wallet's coin-selection and `wait_for_proof` never validate a
candidate UTXO against current network truth before committing 300s to waiting on it, and
nothing in the wallet reconciles/invalidates a persisted UTXO once its real spend falls
outside whatever window was last rescanned** — is a real, generally applicable gap, not an
artifact confined to this test fixture. A fresh interactive-GUI wallet with a shorter
history would be less likely to hit it, but nothing prevents the same class of divergence
recurring given enough wall-clock time or SPV rescan/import edge cases.

## 7. TC-005 (`test_tc005_create_top_up_asset_lock`) — skipped

Not run. `fixtures::shared_identity()` (its precondition) requires a successful identity
registration, which itself funds via the same framework wallet's coin-selection path
implicated above — a second data point would very likely reproduce the identical mechanism
rather than add new information, and each attempt costs up to ~300s+ real wall-clock time.
Given §3-§6 already gives a conclusive, independently-verified mechanism, I judged the
marginal value low relative to the time cost and stopped here.

## 8. Key evidence artifacts

- Run 3 full log (clean, instrumented repro — 1316 lines):
  `/data/tmp/backend-e2e-tc004-tqNtw6.log`
- Run 2 full log (harness-init panic, collateral damage from my kill of run 1):
  `/data/tmp/backend-e2e-tc004-96Ggen.log`
- rust-dashcore pin: `647fa9820f3614090e4e5f5f2b709961d68e538b`
  (cached checkout: `~/.cargo/git/checkouts/rust-dashcore-c6b13647c01f74b9/647fa98`)
- platform pin: `c2135800` (cached checkout:
  `~/.cargo/git/checkouts/platform-7a21f318038a582f/c213580`)

## 9. Minor doc/behavior mismatches noticed in passing (not the main finding)

- `tests/backend-e2e/README.md` documents the workdir as "keyed by git revision" (e.g.
  `/tmp/dash-evo-e2e-testnet-abc1234`) and a 180s spendable-balance timeout at init. Actual
  code (`harness.rs`) uses a fixed base path with a numbered-slot fallback (no git-hash
  component) and a 30s spendable-balance timeout. Neither is a functional bug, but the
  README no longer matches the code.

---

## 10. Follow-up experiment (same session, later): cold-cache wipe — headline finding

**Prompted by a user hypothesis**: is §6's "stale UTXO" scoped to this long-lived,
never-wiped E2E workdir (cache/hygiene issue), or a real architectural gap? Tested by
backing up (renaming, not deleting) the primary workdir
(`/tmp/dash-evo-e2e-testnet` → `.bak-20260707T084128Z`, later preserved as
`.cold-synced-20260707T084959Z`) and forcing a genuine cold init. Confirmed genuine:
`BlockHeadersManager initialized at height 0` (not the cached tip), fresh
`Registered framework wallet` (not "already registered"), and a real filter/block
re-matching pass (323→399 historical blocks independently reprocessed across the session).

**Result — far more damning than the single stale UTXO in §4.** After a full genesis
rescan, `verify_framework_funded` (`framework/funding.rs:77`) reported the framework
wallet's REAL spendable balance as **36,908,682 duffs (≈0.369 DASH)** — against the
**15,082,365,339 duffs (≈150.8 DASH)** the stale warm cache had been reporting across every
run in §1-§9. **The wallet's apparent balance was >99.9% phantom.** This isn't one bad UTXO;
essentially the entire locally-tracked balance for this wallet does not exist on the real
chain. This independently and much more broadly corroborates the self-reinforcing
phantom-chain mechanism already on record in MemCan from the 2026-07-01 session
(`dispatch_local` injecting unconfirmed self-broadcasts as spendable, with no
acceptance/rejection reconciliation) — this session's cold rescan is the first time that
mechanism's *cumulative* damage over ~54 days of unreconciled local state has been measured
directly rather than inferred from one transaction.

Practical consequence: the harness's own 10 DASH minimum-funding gate (`MIN_BALANCE_DUFFS`)
now correctly fails fast on the truly-synced workdir — meaning further live testing needed
either faucet/manual funding or continuing against the (known-tainted) warm cache. The
cold-synced workdir was preserved (not deleted) for follow-up once real funds land; a fresh,
independently Insight-verified never-used receive address (`yYqF93Sonfe1ETRPim5vATsNdTa4qztyXf`
— `balance:0, totalReceived:0, txApperances:0`) was handed off for manual top-up.

## 11. Addition 1 — repeat-run/restart stability: BLOCKED, not inconclusive

Plan was 2-3 more `cargo test` invocations (each a genuine fresh process — satisfies
"restart between runs") against the now-cold-synced workdir. **Could not be completed as
scoped**: every subsequent invocation hits the same `verify_framework_funded` panic before
reaching any asset-lock code, since the real balance (0.369 DASH) is below the hard-coded
10 DASH gate. This is not "inconclusive" — see §10's headline finding and §13's synthesis
below, which make repeat-run testing on *this* wallet moot until it holds real funds:
literally any transaction it builds right now is provably phantom (§13), so "does it pass
consistently" has a deterministic answer (no, never, until re-funded for real) rather than
an intermittent one.

## 12. Addition 2 — late-added wallet (permanent test `TC-012`)

Added `test_tc012_create_registration_asset_lock_late_added_wallet` to
`tests/backend-e2e/core_tasks.rs` (committed; `cargo +nightly fmt` and
`cargo clippy --all-features --all-targets -- -D warnings` both clean). Uses
`create_funded_test_wallet` (registers a wallet *into* an already-running, already-synced
SPV client — the opposite of the framework wallet, which is registered *before*
`backend.start()` in `BackendTestContext::init`, see §14) then attempts
`CreateRegistrationAssetLock` from it, same pattern as TC-004.

Three attempts, each informative in a different way:

| Attempt | Wait strategy | Result | Time |
|---|---|---|---|
| v1 | none (relied on `create_funded_test_wallet`'s own "spendable" wait) | `AssetLockTransaction("... Coin selection error: No UTXOs available for selection")` | 12.24s |
| v2 | added explicit poll on `.confirmed` (not `.spendable()`), 180s bound | Timed out: `confirmed=0` for the full 180s | 215.11s |
| v3 | same poll, bumped to 420s bound | Timed out again: `confirmed=0` for the full 420s — longer than one average Dash block (~2.5 min) | 432.80s |

v1's failure exposed a real, separate harness/product balance-classification mismatch (see
`DetWalletBalance::spendable()` in `src/wallet_backend/snapshot.rs`: `confirmed +
unconfirmed`, explicitly a UI-display-only heuristic per that file's "FUND-SAFETY MANDATE"
banner) — `create_funded_test_wallet`'s wait is satisfied by a plain unconfirmed mempool
deposit, but the real upstream asset-lock coin-selector requires strictly confirmed/IS-locked
funds and correctly refuses the unconfirmed ones (`Coin selection error: No UTXOs available`).
That refusal is *correct*, conservative behavior on the coin-selector's part — not itself a
bug — but it meant v1 wasn't actually testing Addition 2's real question yet, so TC-012 was
revised (v2/v3) to wait on `.confirmed` specifically before attempting the asset lock.

v2 and v3 then hit something worse: **the funding transaction itself never confirmed, at
all, in 420 seconds — three times longer than an average Dash block.** That is not
plausible IS-lock variance; it demanded direct verification.

## 13. Synthesis: the funding chain itself is phantom, generation after generation

Cross-checked both `create_funded_test_wallet` funding txids (one per TC-012 attempt) against
Insight, plus each one's *own* input:

| Tx | Role | Insight |
|---|---|---|
| `0d3447479d6782005897eb9d2bb8d104de36aaf0312c602e92e9ba23cb1b3b59` | v2's framework→test-wallet funding tx (0.02 DASH) | **Not found** |
| `1501021799c7087f0dd64a0c5b58d67dd42932d13068cda926daa04bfa1a7071` | v3's framework→test-wallet funding tx (0.02 DASH) — **spends `0d344747…:1`, v2's own change output** | **Not found** |

**v3's funding transaction spends v2's funding transaction's change output — and neither
transaction ever reached the real network.** This is the exact self-reinforcing phantom
chain mechanism from the 2026-07-01 MemCan record, caught live, two generations deep, in a
completely different code path (`CoreTask::SendWalletPayment`, ordinary wallet-to-wallet
funding — not even the asset-lock builder) than TC-004's asset-lock-specific repro. The
framework wallet's coin-selection is currently incapable of producing a transaction that
reaches the real network, *for any purpose* — asset-lock creation, or a plain payment.
`confirmed` staying at 0 for 420s in TC-012 isn't a timing gap; it's the deterministic
consequence of funding a wallet from a transaction that never left this machine.

**This reframes Addition 2's answer.** The late-added test wallet does *not* fail because
of its own history (it has none — a fresh 12-word mnemonic can't have a stale UTXO). It
fails because it was funded *from* the framework wallet, whose own coin-selection is already
thoroughly poisoned. The discriminating variable isn't "when was this wallet registered
relative to SPV startup" (the original framing) — it's "does this wallet's balance trace
back to a real, network-accepted transaction, or to a chain of purely-local phantom
self-broadcasts." A late-added wallet funded from a genuinely clean source (e.g. a real
faucet drip, or the user's pending manual top-up to the address in §10) would very plausibly
behave differently — that comparison is the natural next step once real funds land, and is
a cheap re-run of the already-committed TC-012 once they do.

## 14. Init-ordering check (requested, answered from code, not requiring a live repro)

`BackendTestContext::init` (`framework/harness.rs`) registers the framework wallet
(`register_wallet_with_retry`, ~line 344) **before** `ensure_wallet_backend` /
`backend.start()` (~line 373-393) — i.e. its addresses are part of the SPV client's very
first sync pass. `create_funded_test_wallet` (~line 520) registers a new wallet **after**
the backend is already running, requiring a live bloom-filter rebuild
(`Wallet monitor revision changed, rebuilding bloom filter` — observed in every TC-012 log)
to pick it up. This asymmetry is real, but §13 shows it isn't what's driving the current
failures — both an "early" wallet (framework) and a "late" wallet (TC-012's) are equally
unable to produce a transaction the network accepts, because the *funding source* is the
same poisoned framework wallet either way.

Aside: `create_funded_test_wallet` always passes `WalletOrigin::Imported` (full genesis
filter-matching pass) even though every call generates a brand-new, guaranteed-empty
12-word mnemonic — `WalletOrigin::Fresh` (birth height = current tip, per
`model/wallet/birth_height.rs`'s own documented policy: "a freshly generated phrase cannot
have prior deposits") would be both correct and cheaper. Observed cost: every TC-012 attempt
paid a multi-hundred-thousand-filter re-match pass (e.g. "Filters: Syncing 1364999/1510083")
for a wallet that can, by construction, never match anything. Not a correctness bug, but an
avoidable per-test-wallet tax worth a follow-up.

## 15. Updated verdict

Unchanged at the headline level — **STILL BROKEN** — but the mechanism is now understood
far more precisely than the original 2026-07-01 doc or even §1-§9 of this one: it is not a
single bad UTXO or a confirmation-timing race. The framework wallet used by this E2E suite
is currently running on an entirely self-generated, self-reinforcing chain of phantom
transactions that has never been reconciled against real chain state, to the point that
>99.9% of its apparent balance does not exist on-chain, and it is currently incapable of
producing *any* transaction — asset-lock or plain payment — that the real network accepts.
Every symptom observed this session (TC-004's FinalityTimeout, TC-012's permanent
zero-confirmation) is a direct, provable consequence of that one fact, not independent bugs.

---

## 16. Real-money follow-up (same session, later): a genuine, clean deposit does not fix it

At the user's request, sent a real manual top-up (not a faucet) to the framework wallet's
verified-unused address from §10 (`yYqF93Sonfe1ETRPim5vATsNdTa4qztyXf`). Confirmed via
Insight within seconds: txid `ad9b30b831de7d2286a1bf9784d5a4e3e06e8cc53af2842376476fee2899931b`,
**20 DASH**, genuinely InstantSend-locked (`"txlock":true`; `blockheight:-1` — not yet
block-mined, but IS-lock is the same finality grade DET's coin-selector accepts). A real,
clean, node-observed deposit, unlike every transaction in §4-§13.

Swapped the preserved cold-synced workdir (§10) back to the primary path and reran
`test_tc004_create_registration_asset_lock` — a "repeat run" per Addition 1, now against a
wallet holding real money. **Still `FinalityTimeout`**, same shape as ever:
`WalletBackend { source: FinalityTimeout(OutPoint { txid: 0xfa5b15ab…, vout: 0 }) }`,
307s total.

The coin-selector did not even reach for the clean 20 DASH deposit — it built the new
asset-lock transaction on a *different* input entirely:
`5c43cd895754b925234950f5e94a7cdfd73b315a6d2eef9eee1b44de03d58de7:1` (value 77.99295932
DASH). Checked on Insight:

| Item | Insight |
|---|---|
| `5c43cd8957…:1` — the input our new asset-lock tx spent | **Confirmed**, block 1474688, 35406 confirmations (~61 days old). But: `"spentTxId":"f9ca52d513f2801a2aec9d222f3d958748254ffb7a4532c6c44b22a111b638c7","spentHeight":1474746"` — **spent for real, 58 blocks after it was created, ~61 days ago.** |
| `fa5b15ab98…` — our new asset-lock tx | **Not found** — same doomed-double-spend fate as every prior attempt. |

**This is a materially different, and more serious, finding than §10's "the whole balance
is phantom."** §10 showed a supposedly-complete cold rescan settling on a real balance of
0.369 DASH — implying the rescan mechanism itself works and the problem was accumulated,
un-reconciled warm-cache staleness. This run shows the opposite: given more elapsed
wall-clock time to keep reconciling (the same workdir, resumed ~35 minutes later,
uninterrupted), the *same, previously-cold-synced* wallet settled on a spendable balance of
**22,998,547,073 duffs (≈230 DASH)** — logged as "sufficient" at the very first balance
check of this run, before this run broadcast anything itself — and picked at least one
UTXO for its next transaction that an independent full node shows was genuinely spent
~35,406 confirmations ago. A resync that is allowed to run longer does not converge on
truth; it reintroduces (or never actually eliminated) at least this stale entry. This
demotes the "cache/hygiene" explanation from §6/§10: the defect is not a property of an
unreconciled warm cache versus a clean cold one — it reproduces after a from-genesis rescan
that was given ample time to complete, on a different historical UTXO each time. The
underlying spend-detection/reconciliation logic in `platform-wallet`/`key-wallet` itself
does not reliably mark this wallet's own historical outputs as spent, regardless of how
"cold" or complete the resync is.

**Practical consequence for Addition 1/2's clean-funding comparison:** routing real money
*through the existing, long-lived framework wallet* does not isolate a clean signal —
its coin-selector can always reach past the fresh deposit into the same poisoned history.
A genuinely clean comparison needs a **brand-new wallet funded directly** (bypassing the
framework wallet as an intermediary), so it has zero prior history to reconcile, correctly,
or incorrectly. That is a cheap follow-up (fund a fresh address directly, then rerun
`TC-012` pointed at that wallet) but was not completed in this session for time.

## 17. Final updated verdict

**STILL BROKEN — confirmed with real money, in a genuinely cold-synced context, against a
different historical UTXO than any prior attempt.** The original H1 (local spendability
diverges from network truth) is now established at three independent depths: (1) a single
stale UTXO in a warm, never-wiped cache (§4); (2) the wallet's *entire* apparent balance
being >99.9% phantom immediately after a cold rescan (§10); and (3) a *different* stale,
genuinely-spent-61-days-ago UTXO surfacing again after that same cold-synced wallet was
given more time to reconcile and received a real, clean, IS-locked deposit (§16). No amount
of waiting, re-syncing, or adding real funds to this wallet has produced a passing run.
Fixing this requires correcting the reconciliation/spend-detection defect itself — this
wallet's local UTXO index cannot currently be trusted to converge on real chain state no
matter how it is refreshed.

---

## 18. Crate attribution — where does the defect actually live?

Two upstream repos are in play (`Cargo.lock`): `key-wallet` + `dash-spv` (both v0.45.0,
`dashpay/rust-dashcore@647fa982`) and `platform-wallet` (v4.0.0,
`dashpay/platform@c213580`, branch `dash-evo-tool`). Read the actual source at both pinned
revisions (not just log-target prefixes).

### 1. Where the Confirmed/Unconfirmed classification happens

`key-wallet/src/wallet/balance.rs` — `WalletCoreBalance` is a plain struct; both buckets are
documented as spendable, the split is display-only (`spendable() = confirmed + unconfirmed`,
lines 52-55). The bucket a UTXO lands in is decided in
**`key-wallet/src/managed_account/managed_core_funds_account.rs::update_balance`
(~line 525-543)**:

```rust
} else if utxo.is_confirmed || utxo.is_instantlocked || utxo.is_trusted {
    confirmed += value;
} else {
    unconfirmed += value;
}
```

`is_confirmed` / `is_instantlocked` are set when a UTXO is first inserted, from the
transaction's `TransactionContext` (`update_utxos`, ~line 241-243, same file). `is_trusted`
is computed just above (~line 182-195) as a recursive Bitcoin-Core-`IsTrusted`-style check:
a self-send's change is only trusted if *every* input it spends is itself
confirmed/IS-locked/already-trusted.

### 2. Does `platform-wallet` trust this as-is, or re-derive it?

**Trusts it as-is — no independent tracking layer.** `platform-wallet`'s
`AssetLockManager::build_asset_lock_transaction`
(`packages/rs-platform-wallet/src/wallet/asset_lock/build.rs`) calls straight into
`info.core_wallet.build_asset_lock_with_signer(...)` — `key-wallet`'s own method
(`key-wallet/src/wallet/managed_wallet_info/asset_lock_builder.rs:256`), which builds via
`TransactionBuilder::set_funding(...).require_final_inputs().build_signed(...)`.
`require_final_inputs` (`transaction_builder.rs:317-318`) filters candidate UTXOs to
`is_confirmed || is_instantlocked` — key-wallet's own flags, read directly, no
re-verification. Likewise `wait_for_proof`
(`packages/rs-platform-wallet/src/wallet/asset_lock/sync/proof.rs`) reads
`key_wallet`'s `TransactionRecord`/`TransactionContext` off the account's own transaction map
(`a.transactions().get(&out_point.txid)`) directly. Platform-wallet has no second opinion
anywhere in this path — if key-wallet's UTXO/transaction bookkeeping is wrong, platform-wallet
has no way to notice.

### 3. Does the landed fix (417d61da / #836) touch the same code, or a separate guard?

**Same file, adjacent code, but a different bug than §16's.** `git show 417d61da` touches
exactly `managed_core_funds_account.rs` (the `is_trusted` computation, made recursive —
previously a flat `has_owned_input && change_addr` check that any self-send could satisfy
regardless of whether its own inputs were final) and `asset_lock_builder.rs`
(adds `.require_final_inputs()` to both asset-lock builders). This closes the loophole where
a chain of *unconfirmed* self-sends could compound trust indefinitely (the original H1/§3-§5
symptom, and the mechanism behind the doc's early phantom-chain sightings).

**It does not touch, and cannot fix, §16's failure mode.** §16's stale UTXO
(`5c43cd8957…:1`) was genuinely `is_confirmed = true` — a real, once-correct block
confirmation flag, not an `is_trusted` mempool inference — so it already satisfies
`require_final_inputs` and sails past 417d61da's guard entirely. The actual defect there is
in the **removal** side of the same function, `update_utxos`'s spent-outpoint loop
(same file, lines 251-267):

```rust
self.reservations.release(tx.input.iter().map(|input| &input.previous_output));
for input in &tx.input {
    self.spent_outpoints.insert(input.previous_output);
    if self.utxos.remove(&input.previous_output).is_some() { ... "Removed spent UTXO" ... }
}
```

This only fires when `update_utxos` is *called* for the spending transaction — i.e. only if
some upstream layer already decided that transaction was relevant to this account. The
function's own author-written comment a few lines above (lines 211-217, present in this file
both before and after 417d61da — untouched by that fix) already flags exactly this class of
risk:

> "Check if this outpoint was already spent by a transaction we've seen. This handles
> out-of-order block processing during rescan... TODO: This is mostly needed for wallet
> rescan from storage — there is a timing issue with event processing which might lead to
> invalid UTXO set / balances. There might be a way around it."

§16 is a live instance of exactly that acknowledged, still-open gap: a genuinely-once-real
`is_confirmed` UTXO that was later spent for real (Insight: block 1474746, 35406
confirmations ago) never got removed from this wallet's local `utxos` map, even after a
from-genesis rescan given ample time to complete.

### 4. Verdict

**Primary defect: `key-wallet` (`dashpay/rust-dashcore`), file
`key-wallet/src/managed_account/managed_core_funds_account.rs`, function `update_utxos`
(spent-outpoint removal loop, lines ~251-267; the acknowledged-but-unfixed rescan/event-timing
TODO sits at lines 211-217 in the same function).** This is where a UTXO's local record
should be — but sometimes isn't — invalidated when the network genuinely spends it.

- **`platform-wallet` is not independently at fault.** It correctly gates asset-lock funding
  on `require_final_inputs` (post-417d61da) and has no separate bookkeeping to get wrong —
  it fully and reasonably trusts `key-wallet`'s flags. The architecture (platform-wallet
  as a thin consumer of key-wallet's UTXO/balance state) is sound; the state it consumes is
  sometimes incorrect.
- **`dash-spv` is a plausible contributing factor, not conclusively pinned.** For
  `update_utxos` to run at all for a given transaction, some upstream layer must first decide
  that transaction is relevant to this account/wallet (address or outpoint match) and hand it
  over — that relevance-matching lives either in `key-wallet`'s own `wallet_checker.rs` or
  further upstream in `dash-spv`'s compact-filter/block-fetch pipeline. Tracing that boundary
  conclusively (e.g. instrumenting exactly why block 1474746's spend of `5c43cd8957…:1` never
  reached this account's `update_utxos`) would need live debugging beyond a source read and
  wasn't done here — flagging as the natural next step for whoever picks up the upstream fix,
  but not required to file the primary issue: `key-wallet`'s own code already documents the
  risk class in the exact function where §16's symptom originates.
