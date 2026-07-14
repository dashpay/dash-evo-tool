# Masternode-identity upgrade test — spec

## Goal

Prove that a masternode (evonode) **identity added as a standalone identity in
DET 0.9.3** survives an in-place upgrade to a newer DET — the identity, its keys,
and its on-chain state must all still be usable after the app and the platform
network are upgraded, the way a real user's install would be.

## Versions under test

| Role | DET | pin |
|------|-----|-----|
| baseline (old) | `v0.9.3` — commit `3268b736` | dash-sdk platform rev `29f7492e` (platform 2.1.0, grovedb 3.1) |
| upgrade (new)  | PR #887 `fix/qa-followups-885` — commit `8a47745a`, base `feat/legacy-identity-migration` | dash-sdk platform rev `93b967f9` (platform 4.0) |

Network: local `dashmate` group, **3 masternodes**. Started on dashmate **v3**
(protocol 11), then upgraded in place to **v4** (protocol 12) — mirroring a real
mainnet protocol upgrade between the two DET versions.

## What we did

1. **Baseline on 0.9.3 (v3 network / protocol 11).** Loaded an evonode identity
   by proTxHash + owner/voting/payout WIFs (for a masternode the platform owner
   identity id *is* the proTxHash). Created a DET wallet, funded it from the
   local_seed Core wallet, topped the identity up via asset lock, and **withdrew
   credits to L1 Dash** — confirmed the asset-unlock landed on the Core chain.
   Snapshotted the DET profile (`backup-1` post-load, `backup-2` post-withdrawal).

2. **Upgraded the network v3 -> v4 in place.** `dashmate@4 group start` migrated
   the config and images; validators signalled protocol 12, which activated at
   the next epoch boundary. From then on Drive emits `GroveDBProof::V1`, which
   0.9.3's SDK cannot decode (expected — this is why the app must be upgraded).

3. **Ran the new DET (PR #887) on a copy of the 0.9.3 profile.** The DB migrated
   cleanly (schema v11 -> v38, no data loss) and the evonode identity appeared in
   the new **Masternodes** section: Active, all three key roles (Voting / Owner /
   Payout) and the derived voter identity intact.

## Results

- **Migration: pass.** Identity + keys + alias survive the 0.9.3 -> PR-887 DB
  migration.
- **On-chain liveness: pass.** Direct DAPI `getIdentity`/`getIdentityBalance`
  (bypassing DET) shows the identity live on protocol 12 with a real, growing
  balance — proof the migrated reference points at the genuine on-chain identity.
- **Key functionality: pass.** DET's *Key Info -> Sign Message* signs a known
  message with the migrated **owner** and **payout (TRANSFER)** keys; each
  signature (a) recovers to the key's expected address, (b) is byte-identical to
  what `dashd signmessage` produces with the genuine private key (deterministic
  RFC6979), and (c) is accepted by `dashd verifymessage`. This proves the private
  key material survived migration and still signs correctly.

## Known limitation (candidate finding for the PR)

A **withdrawal through the new DET cannot be exercised on a local network.** The
new DET verifies platform proofs *trustlessly* via its own SPV-synced masternode
list (`SpvProvider` in `src/context_provider.rs`; no core-RPC fallback — the old
`core_backend_mode` was dropped, migration v38 notes "SPV-only now"). That SPV
masternode sync uses the DIP24 `qrinfo` path, which needs an `llmq_test_dip0024`
rotation quorum. That quorum is **type 103, size 4, minSize 4** (per dash-spv
`test_utils/masternode_network.rs`), so it cannot form on a 3-masternode local
network, and regtest exposes no param to shrink it (only `llmq_test`,
`llmq_test_instantsend`, `llmq_test_platform` are resizable). Result: the new
DET's `masternodes_ready()` never becomes true, every DAPI address gets banned
("servers unreachable"), and no proof-verified op (balance refresh, withdrawal)
can run locally.

DET 0.9.3 was unaffected because it fetched quorum keys straight from its local
Core node over RPC (`core.get_quorum_public_key`), which has no dip0024 dependency.

**Open question for the PR authors:** is a standard `dashmate` local network a
supported target for the new SPV-only verifier, or should the SPV masternode sync
fall back to the legacy `mnlistdiff` path (or accept trusted core quorum data)
when no rotation quorum exists? Until then, an end-to-end withdrawal has to be
tested on devnet/testnet (which have rotation quorums), and the **key-signing
check above is the local substitute** that proves the migrated keys are usable.

## How to reproduce

See `harness/README.md`. The scripts regenerate the network, keys, and profiles
locally; secrets (private keys, wallet DBs) are gitignored and never committed.
