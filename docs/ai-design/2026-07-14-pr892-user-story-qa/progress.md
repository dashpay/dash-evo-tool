# PR892 User-Story QA — Progress Checklist

Tracks completion of every story in `docs/user-stories.md` against the PR892 build. One line per story.

Verdicts: PASS / FAIL / BLOCKED (reason) / N/A (Gap/Superseded/Removed — not implemented, no testing needed).

**Note:** the source brief for this campaign referenced 152 stories across categories WAL/SND/ALK/IDN/DPN/DPY/TOK/DOC/DEV/NET/MCP/UX/IDH/MN. The actual `docs/user-stories.md` at the PR892 base (`v1.0-dev`) contains 123 stories (112 `[Implemented]`, 11 `[Gap]`) across only WAL/SND/ALK/IDN/DPN/DPY/TOK/DOC/DEV/NET/MCP — no UX, IDH, or MN category exists in this document version. Masternode/evonode aspects are covered under IDN-003 and DEV-006. Proceeding with the document as it actually exists.

**Totals:** 123 stories total — 112 `[Implemented]` (to test), 11 `[Gap]` (N/A, skip).


## WAL

- [x] WAL-001: Create a new wallet — PASS
- [x] WAL-002: Import wallet via mnemonic — PASS
- [x] WAL-003: Import single private key — PASS (send-from-SK is a documented product limitation, not a bug)
- [x] WAL-004: Switch between wallets — PASS (per-network isolation noted, not a defect; multi-wallet switching confirmed)
- [x] WAL-005: Rename a wallet — FAIL (Rename button is completely inert on both HD and SK wallets)
- [x] WAL-006: Lock and unlock wallet — FAIL (Lock works; Unlock never opens a password prompt — self-lockout bug)
- [x] WAL-007: Remove a wallet — FAIL (confirmation prompt missing for single-key wallets; present for HD wallets)
- [x] WAL-008: View wallet balances — PASS (Default view does not actually simplify the Wallet screen — UX gap noted)
- [x] WAL-009: View fiat equivalent of balances — N/A (Gap, not implemented)
- [x] WAL-010: Generate receive address — PASS
- [x] WAL-011: View address table — PASS
- [x] WAL-012: View and export private keys — PASS
- [x] WAL-013: View SPV sync status — PASS
- [x] WAL-014: Label addresses — N/A (Gap, not implemented)
- [x] WAL-015: Create throwaway wallet without mnemonic backup — N/A (Gap, not implemented)
- [x] WAL-016: View transaction history — PASS (PR892 cold-boot regression test confirmed fixed)
- [x] WAL-017: Fund Platform address from wallet — FAIL (asset-lock coin selection: "No UTXOs available for selection" despite funded wallet)
- [x] WAL-018: Fund Platform address from asset lock — BLOCKED (no asset lock can ever be created due to WAL-017 bug)
- [x] WAL-019: Transfer credits between Platform addresses — BLOCKED (no Platform address ever holds balance due to WAL-017 bug)
- [x] WAL-020: Withdraw from Platform address to Core — BLOCKED (same root cause as WAL-019)
- [x] WAL-021: Navigate wallet accounts via tabs — PASS
- [x] WAL-022: View system accounts in developer mode — PASS (System tab gated on "not Default view", i.e. Expert or Developer)
- [x] WAL-023: Collapsible transaction history — PASS
- [x] WAL-024: Collapsible balance breakdown — PASS

## SND

- [x] SND-001: Send Dash to an address — PASS (nav confirmed; full E2E send now completed — but no confirmation dialog appears before broadcast, see SND-005)
- [x] SND-002: Send Dash from single-key wallet — FAIL (send disabled for SK wallets; explicit typed error `SingleKeyWalletsUnsupported`)
- [x] SND-003: Receive Dash with QR code — FAIL (Receive button inert, no QR shown)
- [x] SND-004: Send to a DPNS username — N/A (Gap, not implemented)
- [x] SND-005: See fee estimate before confirming send — FAIL (no fee estimate or confirmation dialog anywhere pre-broadcast; Max silently deducts an undisplayed fee)
- [x] SND-006: Send to multiple recipients — PASS (add/remove recipients, single tx broadcast confirmed on-chain)
- [x] SND-007: Shield DASH from Core wallet — FAIL ("Invalid output address" on submit; root cause disclosed in-app as "Shielded sending is not available on this network yet")
- [x] SND-008: Top up identity from Send screen — BLOCKED (no identity exists yet — IDN not run; Identity-destination UI recognition partially verified)
- [x] SND-009: Shield credits from Platform address — BLOCKED (WAL-017: no Platform address ever holds balance)
- [x] SND-010: Withdraw from shielded pool to Core address — BLOCKED (shielded balance always 0; no "Shielded Pool" source option exposed in Send screen)
- [x] SND-011: Transfer identity credits to another identity — BLOCKED (no identity exists yet — IDN not run)
- [x] SND-012: Withdraw identity credits to Core address — BLOCKED (same reasoning as SND-011)
- [x] SND-013: Transfer identity credits to Platform address — BLOCKED (same reasoning as SND-011)

## ALK

- [x] ALK-001: Create an asset lock — PASS (also: differential re-test proves WAL-017's
      coin-selection failure is NOT a global/persistent defect — see scope conclusion in
      `scenarios/ALK.md`; IDN/DPN/DPY/TOK/DOC should NOT be pre-emptively marked BLOCKED)
- [x] ALK-002: View asset lock details — FAIL ("Asset Locks" list never shows a just-created,
      confirmed-usable lock, even after Refresh/renavigation — data is persisted correctly per
      direct SQLite check, this is a UI/cache bug, not a coin-selection issue)
- [x] ALK-003: Recover unused asset locks — BLOCKED (same list-population bug as ALK-002 blocks
      reaching any recovery UI)
- [x] ALK-004: Quick-fund workflow — N/A (Gap, not implemented)

## IDN

- [x] IDN-001: Register a new identity — BLOCKED (wizard/validation confirmed working;
      independent "+Add Key" no-op bug found in Advanced key-selection mode)
- [x] IDN-002: Load existing identity by ID — FAIL (ID+key "Load Identity" button silently
      hangs with zero feedback; sibling tabs on the same screen — "From my wallet", "My
      username" — degrade gracefully with clean typed/generic errors)
- [x] IDN-003: Load evonode/masternode identity — FAIL (same silent-hang defect class as
      IDN-002 on "Load masternode"; ProTxHash format validation and node-type toggle both PASS)
- [x] IDN-004: Top up identity credits — BLOCKED (no identity reachable — see IDN-001/002/003)
- [x] IDN-005: Withdraw credits to Core address — BLOCKED (same reasoning as IDN-004)
- [x] IDN-006: Transfer credits between identities — BLOCKED (same reasoning as IDN-004)
- [x] IDN-007: Add key to identity — BLOCKED (same reasoning as IDN-004)
- [x] IDN-008: View identity keys and details — BLOCKED (same reasoning as IDN-004)
- [x] IDN-009: Refresh identity state — BLOCKED (same reasoning as IDN-004)
- [x] IDN-010: Search identity by DPNS name — BLOCKED (dispatches and fails cleanly on the
      known masternode-list/quorum-sync error, same signature as DEV.md)
- [x] IDN-011: Bulk identity creation — N/A (Gap, not implemented)
- [x] IDN-012: Register identity from Platform addresses — BLOCKED (confirmed implemented and
      correctly gated in source; live Platform-balance cache never populates in this session)
- [x] IDN-013: Top up identity from Platform addresses — BLOCKED (no identity reachable)

## DPN

- [x] DPN-001: Register a DPNS username — BLOCKED (no identity reachable; client-side
      name-format validation + fee estimate confirmed implemented via source)
- [x] DPN-002: View owned usernames — BLOCKED (no identity reachable)
- [x] DPN-003: View active name contests — BLOCKED (no masternode/evonode identity
      reachable — IDN-003)
- [x] DPN-004: View past name contests — BLOCKED (same as DPN-003)
- [x] DPN-005: Vote on contested names — BLOCKED (same as DPN-003; acceptance criteria
      itself requires a masternode/evonode identity)
- [x] DPN-006: Schedule votes — BLOCKED (same as DPN-003)
- [x] DPN-007: Batch voting across contests — BLOCKED (same as DPN-003)

## DPY

- [x] DPY-001: View and edit DashPay profile — BLOCKED (no identity reachable)
- [x] DPY-002: Search DashPay profiles — BLOCKED (no identity reachable)
- [x] DPY-003: Send contact request — BLOCKED (no identity reachable; self-testable via
      second identity in principle, but a first identity can't be established either)
- [x] DPY-004: Accept or reject contact requests — BLOCKED (same as DPY-003)
- [x] DPY-005: View contact list and details — BLOCKED (no identity reachable)
- [x] DPY-006: Send payment to contact — BLOCKED (same as DPY-003)
- [x] DPY-007: View payment history — BLOCKED (no identity reachable)
- [x] DPY-008: Generate DashPay QR code — BLOCKED (no identity reachable)
- [x] DPY-009: Edit contact info — BLOCKED (needs an existing contact; two identities
      unreachable)
- [x] DPY-010: Remove a contact — N/A (Gap, not implemented)
- [x] DPY-011: Auto-accept contact requests — BLOCKED (no identity reachable)

## TOK

- [x] TOK-001: View token balances — BLOCKED (empty state confirmed reachable and correct)
- [x] TOK-002: Search and discover tokens — BLOCKED (confirmed reachable without identity;
      dispatches + fails cleanly on known quorum-sync error)
- [x] TOK-003: Add token by contract or token ID — FAIL (format validation + dispatch both
      work; well-formed-ID request fails but result is silently dropped, zero user feedback)
- [x] TOK-004: Transfer tokens — BLOCKED (no tracked token/identity reachable)
- [x] TOK-005: Create token contract — BLOCKED (live-tested: clean typed error, Advanced
      Options doesn't bypass the identity gate)
- [x] TOK-006: Mint tokens — BLOCKED (no tracked token/identity reachable)
- [x] TOK-007: Burn tokens — BLOCKED (same as TOK-006)
- [x] TOK-008: Freeze and unfreeze token recipients — BLOCKED (same as TOK-006)
- [x] TOK-009: Pause and resume token transfers — BLOCKED (same as TOK-006)
- [x] TOK-010: Destroy frozen funds — BLOCKED (same as TOK-006)
- [x] TOK-011: Claim distributed tokens — BLOCKED (same as TOK-006)
- [x] TOK-012: Set token pricing and purchase tokens — BLOCKED (same as TOK-006)
- [x] TOK-013: Update token configuration — BLOCKED (same as TOK-006)
- [x] TOK-014: Group actions for multi-party governance — BLOCKED (live-tested: clean empty
      states for contract/identity selectors, no crash)
- [x] TOK-015: View available token claims — BLOCKED (same as TOK-006)
- [x] TOK-016: Estimate perpetual token rewards — BLOCKED (no tracked token to estimate for)
- [x] TOK-017: Pay for document operations with tokens — BLOCKED (transitively, via DOC's
      contract-add environment blocker)

## DOC

- [x] DOC-001: Register a new data contract — BLOCKED (live-tested: clean typed "No identities
      loaded" message, no crash)
- [x] DOC-002: Update an existing data contract — FAIL — **application crash**: `.expect()` on
      `get_contracts()` panics on `WalletBackendNotYetWired`; app relaunched, zero persistent
      state lost
- [x] DOC-003: Import and manage contracts — BLOCKED (confirmed reachable without identity;
      dispatches + fails cleanly on known quorum-sync error)
- [x] DOC-004: Query and browse documents — FAIL (dispatches a real query that hangs silently
      forever, with a misleading ever-counting "Querying documents..." progress banner)
- [x] DOC-005: Create a document — BLOCKED (reachable, clean empty state, no crash)
- [x] DOC-006: Replace or update a document — BLOCKED (same as DOC-005)
- [x] DOC-007: Delete a document — BLOCKED (same as DOC-005)
- [x] DOC-008: Transfer document ownership — BLOCKED (same as DOC-005)
- [x] DOC-009: Purchase a document and set document pricing — BLOCKED (same as DOC-005; both
      "Purchase Document" and "Set Document Price" menu items tested)

## DEV

- [x] DEV-001: Decode state transitions — PASS
- [x] DEV-002: View proof request log — FAIL (no UI implementation found; only a failure-only tracing target, no browsable log)
- [x] DEV-003: Inspect ZK proofs — FAIL (Proof deserializer works; GroveSTARK gen/verification deliberately hidden from all UI navigation)
- [x] DEV-004: View document and contract JSON — BLOCKED (Contract deserializer PASS; Document deserializer's contract-loading path blocked by known Testnet masternode-list/quorum-sync issue, see ALK.md)
- [x] DEV-005: View Platform info — FAIL (2/8 sub-tools work — Basic Platform Info, Validator Set Info; rest blocked by known masternode-list-sync issue)
- [x] DEV-006: View masternode list diff — FAIL (no UI implementation found; Masternodes screen only supports per-node load-by-ProTxHash, not list diff/monitoring)
- [x] DEV-007: Check any address balance — BLOCKED (address-format validation PASS; balance fetch blocked by known masternode-list-sync issue)
- [x] DEV-008: Mine blocks on Regtest — BLOCKED (Regtest-only, no regtest node running in this environment)

## NET

- [x] NET-001: Switch networks — PASS
- [x] NET-002: Auto-update from dashmate config — FAIL (no detection/import UI anywhere;
      `.env.example` requires the user to manually run `dashmate config get
      core.rpc.users.dashmate.password ...` and paste it in by hand)
- [x] NET-003: Configure Dash-Qt path — FAIL (`dash_qt_path` exists in the settings model
      with autodetection, but zero UI surface to view/edit/validate it; no `SystemTask`
      variant to update it)
- [x] NET-004: Select theme — PASS
- [x] NET-005: Toggle developer mode — PASS
- [x] NET-006: Select user mode — N/A (Gap, not implemented)
- [x] NET-007: Granular refresh controls — PASS (partial; only 2 modes exist —
      "Core + Platform" / "Platform Only" — not the 3 described in the story text; see note)
- [x] NET-008: Select Core backend mode — FAIL (explicitly retired in code: "chain sync is
      SPV-only now"; no SPV/RPC/Auto selector exists anywhere in the UI)
- [x] NET-009: Toggle ZMQ — FAIL (`disable_zmq` field exists in settings model, zero UI
      surface, no `SystemTask` variant to update it)
- [x] NET-010: Onboarding wizard — PASS
- [x] NET-011: Wipe Platform data — BLOCKED (deliberately not run: destructive/irreversible
      against the campaign's shared, evidence-bearing data dir; the agent permission system
      independently halted the attempt and requires explicit human confirmation — see
      `scenarios/NET.md` and `summary-report.md` for details)
- [x] NET-012: Configure Devnet through the UI — N/A (Gap, not implemented)
- [x] NET-013: Testnet faucet integration — N/A (Gap, not implemented)
- [x] NET-014: Bulk fund addresses — N/A (Gap, not implemented)
- [x] NET-015: Use Dash Evo Tool without a local Dash Core node — PASS (with a UX note:
      the default-view global banner still says "SPV sync failed", leaking jargon the
      story says the everyday-user UI should avoid)

## MCP

- [x] MCP-001: Manage wallets via CLI — FAIL (imported wallets are invisible to every
      subsequent `det-cli` command — `core_wallets_list`/`core_address_create`/
      `core_balances_get` all return "Wallet not found" for a wallet imported by a prior
      process, or even by an earlier `already_imported:true` import in the same process;
      root cause confirmed in source: `ListWalletsTool` reads only the in-memory
      `ctx.wallets` map, which is never hydrated from the DB/vault outside the
      SPV-gated path)
- [x] MCP-002: MCP server access for AI agents — PASS (stdio via `det-cli serve` and HTTP
      via `det-cli headless` both verified: protocol lifecycle, bearer auth, session
      handling, network-mismatch guard, dynamic tool discovery all work correctly; carries
      the same wallet-hydration caveat as MCP-001 but that is a wallet-tooling defect, not
      a transport/protocol defect)
