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

- [ ] IDN-001: Register a new identity
- [ ] IDN-002: Load existing identity by ID
- [ ] IDN-003: Load evonode/masternode identity
- [ ] IDN-004: Top up identity credits
- [ ] IDN-005: Withdraw credits to Core address
- [ ] IDN-006: Transfer credits between identities
- [ ] IDN-007: Add key to identity
- [ ] IDN-008: View identity keys and details
- [ ] IDN-009: Refresh identity state
- [ ] IDN-010: Search identity by DPNS name
- [x] IDN-011: Bulk identity creation — N/A (Gap, not implemented)
- [ ] IDN-012: Register identity from Platform addresses
- [ ] IDN-013: Top up identity from Platform addresses

## DPN

- [ ] DPN-001: Register a DPNS username
- [ ] DPN-002: View owned usernames
- [ ] DPN-003: View active name contests
- [ ] DPN-004: View past name contests
- [ ] DPN-005: Vote on contested names
- [ ] DPN-006: Schedule votes
- [ ] DPN-007: Batch voting across contests

## DPY

- [ ] DPY-001: View and edit DashPay profile
- [ ] DPY-002: Search DashPay profiles
- [ ] DPY-003: Send contact request
- [ ] DPY-004: Accept or reject contact requests
- [ ] DPY-005: View contact list and details
- [ ] DPY-006: Send payment to contact
- [ ] DPY-007: View payment history
- [ ] DPY-008: Generate DashPay QR code
- [ ] DPY-009: Edit contact info
- [x] DPY-010: Remove a contact — N/A (Gap, not implemented)
- [ ] DPY-011: Auto-accept contact requests

## TOK

- [ ] TOK-001: View token balances
- [ ] TOK-002: Search and discover tokens
- [ ] TOK-003: Add token by contract or token ID
- [ ] TOK-004: Transfer tokens
- [ ] TOK-005: Create token contract
- [ ] TOK-006: Mint tokens
- [ ] TOK-007: Burn tokens
- [ ] TOK-008: Freeze and unfreeze token recipients
- [ ] TOK-009: Pause and resume token transfers
- [ ] TOK-010: Destroy frozen funds
- [ ] TOK-011: Claim distributed tokens
- [ ] TOK-012: Set token pricing and purchase tokens
- [ ] TOK-013: Update token configuration
- [ ] TOK-014: Group actions for multi-party governance
- [ ] TOK-015: View available token claims
- [ ] TOK-016: Estimate perpetual token rewards
- [ ] TOK-017: Pay for document operations with tokens

## DOC

- [ ] DOC-001: Register a new data contract
- [ ] DOC-002: Update an existing data contract
- [ ] DOC-003: Import and manage contracts
- [ ] DOC-004: Query and browse documents
- [ ] DOC-005: Create a document
- [ ] DOC-006: Replace or update a document
- [ ] DOC-007: Delete a document
- [ ] DOC-008: Transfer document ownership
- [ ] DOC-009: Purchase a document and set document pricing

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
- [ ] NET-002: Auto-update from dashmate config
- [ ] NET-003: Configure Dash-Qt path
- [ ] NET-004: Select theme
- [ ] NET-005: Toggle developer mode
- [x] NET-006: Select user mode — N/A (Gap, not implemented)
- [ ] NET-007: Granular refresh controls
- [ ] NET-008: Select Core backend mode
- [ ] NET-009: Toggle ZMQ
- [ ] NET-010: Onboarding wizard
- [ ] NET-011: Wipe Platform data
- [x] NET-012: Configure Devnet through the UI — N/A (Gap, not implemented)
- [x] NET-013: Testnet faucet integration — N/A (Gap, not implemented)
- [x] NET-014: Bulk fund addresses — N/A (Gap, not implemented)
- [ ] NET-015: Use Dash Evo Tool without a local Dash Core node

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
