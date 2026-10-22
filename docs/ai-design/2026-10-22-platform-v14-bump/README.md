# Platform protocol 14 support (Platform v5.1-dev pin)

A point-in-time record of PR #1015: DET on Platform `v5.1-dev`
(`033ecaee2f3badc8e04ff8be664062f33516e3e4`) and the protocol 14 features it
adds. It supersedes the open "Devnet protocol seed" item of
[`2026-09-14-platform-4.2-dev-bump`](../2026-09-14-platform-4.2-dev-bump/upgrade-notes.md).

## Which protocol version DET builds for

There are two version sources, and they can disagree while a network moves
from 13 to 14:

- **The SDK's version** (`AppContext::platform_version()`). It starts at
  upstream's per-network `dash_sdk::sdk::min_protocol_version` (13 on mainnet,
  testnet and regtest, 14 on devnet) and ratchets to what verified responses
  report. Transitions are encoded and system contracts cached at this version,
  and the cache reloads when it changes.
- **The network's confirmed version** (`AppContext::platform_protocol_version()`).
  It comes from a proved epoch read (or a proved DPNS fetch). It is `0` until
  confirmed. Feature gates (`context/feature_gate.rs`) read only this one, so
  an unconfirmed network never unlocks a protocol 14 feature.
  `connected_platform_version()` selects that version's tables for estimates.

If a protocol 14 structure is encoded at the protocol 13 table, it keeps its
v14 data (a limited key stays `IdentityPublicKeyInCreation::V1`; the
once-per-identity token rules stay V1). A protocol 13 network then refuses it.
It is never silently downgraded (`backend_task/protocol_13_transitions.rs`).

## Protocol 14 features, all gated on the confirmed version

| Feature | Where |
|---|---|
| Key spending limits and expiry, raising them | `model/identity_key_limits.rs`, Add Key, Key Info |
| Contract-bound authentication keys | `model/identity_key_usability.rs` |
| Once-per-identity token distributions | `model/token.rs`, token creator, Claim |
| Document action fees, confirmed before signing | `model/fee_estimation.rs`, document action screen |
| Contract fee pots and claims | `backend_task/contract_fee_pots.rs`, Fee Pots screen |
| Failed-withdrawal status | `backend_task/platform_info.rs` |

## Decisions

- **Key choice (IDN-024).** Automatic selection keeps only keys that are live,
  held on this device, in scope, and at a security level the action accepts.
  Among those it prefers unlimited keys. The key is picked again when the
  scope changes. The manual chooser leaves out keys bound to another contract
  or document type, but keeps expired and limited keys with a warning.
- **What is confirmed is what is sent.** The confirmations for document action
  fees, token claims and key limit raises build the action when they open and
  block the form behind them. Confirm sends exactly that action.
- **Only proved data gates an action.** Fee pot claims are not blocked on the
  response metadata's epoch, because the proof does not sign it. Platform
  refuses a second claim in the same epoch, with a typed error.
- **Raises only.** `KeyLimitsRaise::check_against`, the last check before
  signing, refuses a lowered limit, and a limit added to a key that has none.

## Known blocker: mainnet proofs

Mainnet nodes still serve the retired protocol 13 proof wire format, which this
SDK rejects. Proof verification stays strict: there is no fallback that skips
or weakens it. The PR stays blocked until an upstream SDK that verifies both
formats, or a server rollout, lands.

## Existing data

The upstream V019 wallet-storage migration adds an asset-lock operation id.
V001–V018 are unchanged. The upgrade is forward only: going back needs a copy
taken before the upgrade.
