# Platform v5.1-dev pin and protocol version tracking

A point-in-time record of the update to Platform `v5.1-dev`
(`063589bf0adbe1a8823a943cee95385b423b0e3f`, 5.0.0-rc.8). The update keeps
today's behaviour on protocol 13 and protocol 14 networks and adds no new
user-facing feature. Protocol 14 features build on it in separate changes.
It supersedes the open "Devnet protocol seed" item of
[`2026-09-14-platform-4.2-dev-bump`](../2026-09-14-platform-4.2-dev-bump/upgrade-notes.md).

## Which protocol version DET builds for

There are two version sources, and they can disagree while a network moves
from 13 to 14:

- **The SDK's version** (`AppContext::platform_version()`). It starts at
  upstream's per-network `dash_sdk::sdk::min_protocol_version` (13 on mainnet,
  testnet and regtest, 14 on devnet) and ratchets to what verified responses
  report. Transitions are encoded at this version. The SDK is no longer pinned
  to a fixed table: a pinned SDK cannot build any protocol 14 transition, and
  cannot follow the versioned identity lookup below.
- **The network's confirmed version** (`AppContext::platform_protocol_version()`).
  It comes from a proved epoch read (or, when that fails, a proved DPNS fetch).
  It is `0` until confirmed. Capabilities (`context/feature_gate.rs`) read only
  this one, through `known_protocol_version`, so an unconfirmed network, or a
  version this build does not define, never unlocks a protocol feature.
  `connected_platform_version()` selects that version's tables for estimates,
  such as the daily withdrawal limit.

## System contracts

`SystemContractsCache` serves the system contracts (DPNS, DashPay, withdrawals,
token history, keyword search) at the SDK's version and reloads them when it
changes. The context provider serves them at the version the proof verifier
passes in (`AppContext::system_contracts_at`), which can be newer than the
SDK's. Each version keeps its own set, so the two lookups do not evict each
other. `try_get` fails rather than serving another version's contracts, so a
proof is never checked against the wrong schema.

## Connection state

- **Epoch adoption.** A proved epoch sets the fee multiplier and raises the
  protocol version (`AppContext::adopt_epoch`). Reads can finish out of order,
  so an epoch older than the adopted one is ignored, and the protocol version
  only moves up.
- **Rebuilding the connection** (`replace_connection`, `rebuild_connection`)
  bumps a generation counter, forgets the adopted epoch, and puts the protocol
  version back to unknown and the fee multiplier to the default, under one
  lock, before the new SDK is published. A read begun on the old connection is
  dropped. Capabilities stay closed until the new connection proves its epoch.

## Protocol 13 guards

- `context/platform_version_guard.rs` compares the protocol 13 client tables
  with a recorded snapshot, so an upstream change to them is reviewed.
- `backend_task/protocol_13_transitions.rs` checks that transitions DET builds
  on a protocol 13 network use encodings protocol 13 decodes.

## Mainnet compatibility

dashpay/platform#5515 versions the non-unique identity lookup: protocol 13
nodes answer the V0 request with the legacy two-proof response, and the SDK
verifies it. Proof verification stays strict; nothing skips or weakens it.

## Existing data

The upstream V019 wallet-storage migration adds an asset-lock operation id.
V001–V018 are unchanged. The upgrade is forward only: going back to an older
build needs a copy of the data directory taken before the upgrade.
