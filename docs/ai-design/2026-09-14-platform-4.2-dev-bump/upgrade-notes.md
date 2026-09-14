# Platform `v4.2-dev` pin upgrade

Reviewed on 2026-09-14. The four Platform dependencies (`dash-sdk`,
`rs-sdk-trusted-context-provider`, `platform-wallet`, `platform-wallet-storage`)
move together from `63cf57f40d0000bf3b2b26026c8fa1c71162852d` (`4.2.0-dev.8`)
to `01d944795e3dc9776fd4d01959df9abc1ea98c87` (`4.2.0-dev.10`, `v4.2-dev` HEAD).
The old pin is an ancestor of the new one: 29 new commits, none dropped.

Transitive moves:

- rust-dashcore: `93260bf39bac5d9d09e89bfb45e9ea3ff7fdcbcd` becomes
  `e4208c90786a6854bd498315bcb571ef24182c15` (0.45.0; five commits, including
  transaction detection and filter-sync fixes).
- GroveDB: 5.0.1 becomes 6.0.0 (`6816457d`). The grovestark dependency keeps
  its own 4.0.0 copy.
- `grovedb-bincode` / `grovedb-bincode-derive` 2.1.0 are added. crates.io
  `bincode` 2.0.1 and 1.3.3 stay in the lock for grovestark and GroveDB 4.0.0
  only.

The full analysis (findings F1–F17, wire-compatibility proof and compile
probe) lives outside the repository in
`/data/artifacts/dash-evo-tool/2026-09-14/`:
`platform-4.2-dev-impact.md`, `platform-4.2-dev-compile-probe.md` and
`platform-3968-dropped-content.md`.

## Compatibility work

| Upstream change | DET change |
| --- | --- |
| [#4635](https://github.com/dashpay/platform/pull/4635): dpp, platform-value and rust-dashcore implement `Encode`/`Decode` from `grovedb-bincode`, which has its own trait identities | `bincode` is aliased to `grovedb-bincode =2.1.0` (`Cargo.toml`). DET's derives and manual codecs over dpp types compile again; import paths are unchanged. |
| [#4625](https://github.com/dashpay/platform/pull/4625): dpp deserialization traits split into trusted and untrusted pairs | The state transition and contract visualizers use the untrusted decoders for pasted bytes. The saved-contract reader (`contract_token_db.rs`) uses the untrusted decoder as well: under the same no-limit config it accepts every valid stored contract, and it does not pre-allocate from length prefixes. |
| [#4645](https://github.com/dashpay/platform/pull/4645): `rewards_in_interval_with_explanation` takes a `&PlatformVersion` | The estimated token-reward query passes the app's active platform version, which selects the version-gated distribution math. |
| [#4708](https://github.com/dashpay/platform/pull/4708) / [#4711](https://github.com/dashpay/platform/pull/4711): new `PlatformWalletError` variants `ShieldedIdentityDebitPending`, `ShieldedRecoveryCorrupted`, `ShieldedRecoveryKeysRequired` | New `TaskError` variants with fixed, actionable messages. The upstream `reason` text stays in the source chain only. Both exhaustive matches name the variants explicitly; identity funding buckets them as `Other` because no identity flow runs a shielded operation. |

## Existing data

**bincode blobs: no migration.** `grovedb-bincode` 2.1.0 is a fork of bincode
2.0.1. On every ordinary (non-`*_untrusted`) entry point DET uses (native
`encode_to_vec`/`decode_from_slice`, `bincode::serde`, and the derive macros),
it produces the same bytes and accepts the same inputs. This was proven from
source: files and function bodies hash-identical, and the fork's derive
output reduces to the original's by literal substitution. See
`## grovedb-bincode wire compatibility` in `platform-4.2-dev-impact.md`. That
covers stored `QualifiedIdentity`/`KeyStorage`, token configurations, the
secret envelopes and every serde sidecar.

The proof is backed by committed fixtures under `tests/fixtures/bincode_pre_bump/`.
They were written with crates.io bincode 2.0.1 on the old pin, before the
dependency switch, and must never be regenerated.

| Fixture | SHA-256 | Guard test |
| --- | --- | --- |
| `qualified_identity.bin` | `d851ca2dc406350932e30341c4047e63f37652727f42eed78557b0c4f388198f` | `model::qualified_identity::bincode_pre_bump_fixture_tests` |
| `contested_name.bin` | `d6fcde1fd65485fcebd02a7ac2af21133ed7115f6ec3b47491f9a6bb4dbd4cc6` | `model::contested_name::bincode_pre_bump_fixture_tests` |
| `token_configuration.bin` | `92c5b82f907273eea4c48fb628c49d969c51402d0bf4ff846348c7824ed9f6e0` | `context::contract_token_db::bincode_pre_bump_fixture_tests` |
| `stored_seed_envelope.bin` | `68c6dce3d8c1674d93986b26751bfb4c40f94d8366a6ec92f70be148f0114579` | `model::wallet::seed_envelope::bincode_pre_bump_fixture_tests` |
| `wallet_meta.bin` | `afce0da03252ff5a26274831f0f2dba97af2de7d8f4a5c3a8d5b20a028f9d6be` | `model::wallet::meta::bincode_pre_bump_fixture_tests` |

Each guard does four things:

- Decodes the fixture through the production reader: `QualifiedIdentity::from_bytes` with its decode limit, and `decode_token_config` for token configurations.
- Requires the whole blob to be consumed and the value to equal the synthetic original.
- Re-encodes the value byte-for-byte.
- Checks that the fixture differs from the legacy-config encoding.

The identity fixture carries every `PrivateKeyData` variant and `PrivateKeyTarget`, voter and operator identities, contract bounds, a disabled key, and a `WalletDerivationPath` with all four `ChildNumber` kinds. The token fixture carries a perpetual `DistributionFunction`, whose upstream `Decode` impl was rewritten. `ContestedName` is persisted through a serde record today; its fixture guards the forked derive macros. All fixture values are synthetic.

**`platform-wallet-storage`: no change.** No migration, schema or secret-envelope
change between the pins.

**Shielded store: additive.** `shielded_pending_spends` gains two
`INTEGER NOT NULL DEFAULT 0` columns via an idempotent `ALTER TABLE`. Older
builds still read the table. Opening the store is stricter: an already-malformed pending-spend row that
cannot be classified now fails the open (`ShieldedRecoveryCorrupted`) instead
of being skipped.

## Deferred

- **Contested DPNS fee label.** At protocol version 14 the contested
  registration fee is 0.1 DASH and consensus requires the exact amount. The
  transition amount already comes from the SDK's platform version, but
  `register_dpns_name_screen.rs` still shows "Cost ≈ 0.2006 Dash". Marked
  `TODO(platform-4.2-dev-bump)` (impact report F4).
- **Devnet protocol seed.** Upstream seeds devnets at protocol version 14
  because lower versions cannot deserialize devnet contracts. DET still seeds
  every network at version 12 (`default_platform_version`). Marked
  `TODO(platform-4.2-dev-bump)` (F5). Visibility is not the blocker:
  `dash_sdk::sdk` is a public module and `min_protocol_version` is a
  `pub const fn` at the new pin (`rs-sdk/src/sdk.rs:70`). What remains is the
  seeding decision itself. The older `TODO(platform#4231)` beside it is a
  separate cleanup; that PR is already merged.
- Other runtime deltas are covered only by network-dependent tests, which were
  not run:
  - rust-dashcore transaction detection and filter sync;
  - the protocol-14 GroveDB proof-envelope floor;
  - grovestark's GroveDB 4.0.0 parsing proofs from GroveDB 6.0 nodes.

## Corrections to the previous notes

`docs/ai-design/2026-09-10-platform-pin/upgrade-notes.md` is kept as a
historical record. Two of its claims are stale:

- [#4585](https://github.com/dashpay/platform/pull/4585) is described as open.
  It merged on 2026-09-11 and is included in this pin (FFI-only; DET does not
  use it).
- [#4587](https://github.com/dashpay/platform/pull/4587) is described as
  carrying the contact-account filter-scan generation fix. That fix was
  removed from #4587 on 2026-09-14 and now lives, unmerged, in
  [#4740](https://github.com/dashpay/platform/pull/4740). Neither pin contains
  it. DET still invalidates scan coverage only for contact accounts it
  creates at bootstrap or unlock. This bump does not change that.

## Validation and limits

- The pre-bump fixture guards passed on the old pin with crates.io bincode
  2.0.1, before the dependency switch, and again on the new pin.
- `cargo check --all-features --all-targets`,
  `cargo fmt --all`, and
  `cargo clippy --all-features --all-targets -- -D warnings` on the new pin.
- `cargo test --all-features --lib`, scoped to the bincode-persistence modules
  (identity blobs including the golden v0.9.3 blob, key storage, seed and
  single-key envelopes, DET k/v, wallet meta, settings, contested names, token
  registry), the fixture guards, and the new shielded error-mapping and
  message tests.
- Not run: the full workspace suite (CI covers it), backend E2E, GUI tests, and
  opening a real user's shielded store. Checks used Linux and synthetic data.
