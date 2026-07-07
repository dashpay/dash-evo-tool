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
