# Platform v5.1-dev pin and protocol version tracking

A point-in-time record of the update to Platform `v5.1-dev`
(`063589bf0adbe1a8823a943cee95385b423b0e3f`, 5.0.0-rc.8). The update keeps
today's behaviour on protocol 13 and protocol 14 networks and adds no new
user-facing feature. Protocol 14 features build on it in separate changes.
It supersedes the open "Devnet protocol seed" item of
[`2026-09-14-platform-4.2-dev-bump`](../2026-09-14-platform-4.2-dev-bump/upgrade-notes.md).

## Which protocol version DET builds for

DET keeps no copy of the network's protocol version. Both values below come
from the current SDK, and a rebuilt connection is a new SDK that starts from
scratch:

- **The SDK's version** (`AppContext::platform_version()`). It starts at
  upstream's per-network `dash_sdk::sdk::min_protocol_version` (13 on mainnet,
  testnet and regtest, 14 on devnet) and ratchets to what verified responses
  report, never to a version this build does not define. Transitions are
  encoded at this version, and its tables serve estimates such as the daily
  withdrawal limit. The SDK is no longer pinned to a fixed table: a pinned SDK
  cannot build any protocol 14 transition, and cannot follow the versioned
  identity lookup below.
- **The network's proven version** (`AppContext::proven_protocol_version()`),
  the SDK's `verified_protocol_version`: the highest version seen in a
  response whose proof and quorum signature were verified, or `None` before
  any. It can be a version this build does not define.
  `AppContext::proven_platform_version()` is `None` then, as it is before
  anything is proven. Capabilities (`context/feature_gate.rs`) read only that,
  so an unproven network, or an unknown version, never unlocks a protocol
  feature. Network info shows the proven number, or says none is confirmed.

## System contracts

`SystemContractsCache` serves the system contracts (DPNS, DashPay, withdrawals,
token history, keyword search) at the SDK's version and reloads them when it
changes. The context provider serves them at the version the proof verifier
passes in (`AppContext::system_contracts_at`), which can be newer than the
SDK's. Each version keeps its own set, so the two lookups do not evict each
other. `try_get` fails rather than serving another version's contracts, so a
proof is never checked against the wrong schema.

## Connection state

- **Fee multiplier.** A proved epoch read sets the fee multiplier, which feeds
  fee estimates only. When the epoch cannot be read, the last known value is
  kept and shown as such, never as a fresh network reading, and a proved DPNS
  fetch still lets the SDK learn the network's version.
- **Rebuilding the connection** (`rebuild_connection`) builds a new SDK over
  the same network, so nothing about the chain changes. The new SDK has proven
  no version, so capabilities stay closed until a proved response carries
  one; the rebuild reads the epoch right away for that, and the read refreshes
  the fee multiplier.

## Mainnet compatibility

dashpay/platform#5515 versions the non-unique identity lookup: protocol 13
nodes answer the V0 request with the legacy two-proof response, and the SDK
verifies it. Proof verification stays strict; nothing skips or weakens it.

## Existing data

The upstream V019 wallet-storage migration adds an asset-lock operation id.
V001–V018 are unchanged. The upgrade is forward only: going back to an older
build needs a copy of the data directory taken before the upgrade.
