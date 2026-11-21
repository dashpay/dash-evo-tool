# Project profile — dashpay/dash-evo-tool

Read by AI reviewers and raters (claudius `severity` skill, `system-rating.md` § Project profile). Claims are confirmed by the maintainer (2026-11-21) or measured. Sources: maintainer decisions on the PRs named; repo paths are relative to the root. Change this file only through a reviewed PR.

**Project type:** mixed
- `src/**` (except below) → GUI app: desktop Rust + egui, self-custodial wallet and Dash Platform client (AGENTS.md § Architecture Overview).
- `src/mcp/**`, `src/bin/det_cli/**` → tooling/CLI that ships to users and can move real funds; zones apply per finding (AGENTS.md § MCP Server & CLI).
- `.claude/skills/**`, `docs/gui-testing/**`, `tools/**`, `scripts/**` (except `scripts/ci/`) → internal dev/agent tooling, not shipped: "a helper tool, not a product" (#1175); #1173 merged over its bot blockers as "total overkill".
- `.github/workflows/**`, `scripts/ci/**` → CI tooling. Exception: `release.yml`, `flatpak.yml` and `weekly-build.yml` build what every user installs, so rate their integrity at user-base scale.
- `tests/**` → the owning app's type; a test-only defect harms no user. A test or CI gate that does not check what it claims gets fixed (#1200, #1176, #1128).
- `docs/**` (other) → documentation: content errors matter, tool edge cases do not (#1175).

**System & scale:**
- One desktop process per user on Linux, Windows and macOS; no project-operated server (AGENTS.md § Platform Targets).
- Networks: Mainnet, Testnet, Devnet, local regtest. The Everyday User is Mainnet only; Devnet and regtest belong to the Platform Developer persona (docs/personas/).
- Reach (measured 2026-11-21, GitHub release downloads): stable v0.9.3 = 368; recent weekly pre-releases = 373–1108 each. A defect merged to `v1.0-dev` reaches weekly-build users within about 7 days.

**Threat model:**
- In: secrets reaching logs, diagnostics, shell traces or committed files, test code included (AGENTS.md § Never commit secrets; #1203, #1173).
- In: what is signed differs from what the user confirmed (amount, recipient, fee, double send) (#1213, #1220).
- Out: an attacker already running as the same OS user (reads the process environment or the X11 screen, plants symlinks or crafted files in the profile directory) (#1173, #1200).
  - Exception: secrets stored on disk are in scope, but `platform-wallet` storage in dashpay/platform owns them. File such findings upstream; do not work around them in DET.
- Out: hostile Platform data. DET trusts data from Platform and the SDK; validation and proofs are dashpay/platform's responsibility, and DET does not re-implement their checks (#1117, #1176).
- One interactive human per profile; concurrency is that human plus background tasks. Races that need unusual timing and lose no funds are declined (#1162, #1165).

**Critical assets** (most important first):
1. Secret confidentiality: seed, private keys, passwords.
2. Funds.
3. What is signed equals what was shown (WYSIWYS).
4. Local wallet and identity data, and upgrade safety: migrations and the deletion chokepoint.
5. True, actionable user-facing text for the Everyday User (AGENTS.md § Error messages).
6. Client liveness: closing, network switch, refresh freshness.
7. Release integrity, and CI gates that test what they claim.
8. Robustness of internal dev/agent tooling and its docs.

**Accepted risks:**
- AR1 Hardening internal dev tooling against its own operator or a same-user process (#1173).
- AR2 Hand-placed or crafted local files: database symlinks, oversized strings, a transient SQLite `-shm` sidecar (#1200).
- AR3 Devnet-only failures, e.g. a Devnet wipe racing an edit ("devnet reset is a developer problem"; #1165, #1210).
- AR4 Narrow races with no fund loss (#1165, #1162).
- AR5 A form edited through assistive technology behind an open confirmation modal; the send uses the confirmed snapshot (#1213).
- AR6 Limitations owned by platform-wallet, the SDK or rust-dashcore: documented as a Known limitation with an upstream link, not worked around in DET (#1158, #1187, #1160).
- AR7 A full path outside the home directory in diagnostics the user chooses to share; the home directory itself is redacted (#1202).
- AR8 Secrets stored without a password stay in the keyless vault until the user opts in to protection (AGENTS.md § wallet_backend).
- AR9 Behaviour identical to the base branch does not block a PR (#1128, #1165, #1112).

**Anchors** (maintainer outcome; band where a review rated it):
- A1 Fixed in PR, blocking (WYSIWYS): the dispatched send could differ from the confirmation shown (#1213).
- A2 Fixed in its own PR the same day, MEDIUM: Send stayed enabled while a token send was in flight, so a second click sent twice (#1220).
- A3 Fixed, blocking (secrets): raw settings-file content in shareable startup diagnostics; the pre-existing one-line twin was fixed in the same PR (#1203).
- A4 Fixed even in dev tooling, LOW: a typed credential printed under inherited shell tracing (#1173).
- A5 Merged over as "total overkill": a typed credential visible in a dev script's child-process environment (#1173).
- A6 Fixed: UI text promising what the app does not do (#1176).
- A7 Fixed whenever raised: fixed-pattern keys or seeds in tests (project rule, AGENTS.md).
- A8 Declined: bot-blocking database symlink, crafted database, `-shm` sidecar (#1200).
- A9 Declined twice, LOW: a Devnet wipe racing an identity edit (#1210).
- A10 Declined with a Known limitation and an upstream link: a multi-input shield passes the pre-flight, then fails on fee in the pinned platform-wallet (#1187).
