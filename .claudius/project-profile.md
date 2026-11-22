# Project profile — dashpay/dash-evo-tool

Read by AI reviewers and raters (claudius `severity` skill, `system-rating.md` § Project profile). Every claim is confirmed by the maintainer (2026-11-21) or measured; nothing here needs to be checked against PR history. Paths are relative to the repo root. Change this file only through a reviewed PR.

**Project type:** mixed
- `src/**` (except below) → GUI app: desktop Rust + egui, self-custodial wallet and Dash Platform client (AGENTS.md § Architecture Overview).
- `src/mcp/**`, `src/bin/det_cli/**` → developer-level tooling (MCP server and CLI). It can still move real funds, so fund and secret gates apply per finding.
- `.claude/skills/**`, `docs/gui-testing/**`, `tools/**`, `scripts/**` (except `scripts/ci/`) → internal dev/agent tooling, not shipped.
- `.github/workflows/**`, `scripts/ci/**` → CI tooling. Exception: `release.yml`, `flatpak.yml` and `weekly-build.yml` build what every user installs, so rate their integrity at user-base scale.
- `tests/**` → the owning app's type; a test-only defect harms no user. A test or CI gate that does not check what it claims gets fixed.
- `docs/**` (other) → documentation: content errors matter, tool edge cases do not.

**System & scale:**
- One desktop process per user on Linux, Windows and macOS; no project-operated server (AGENTS.md § Platform Targets).
- Networks: Mainnet, Testnet, Devnet, local regtest. The Everyday User is Mainnet only; Devnet and regtest belong to the Platform Developer persona (docs/personas/).
- Reach (measured 2026-11-21, GitHub release downloads): stable v0.9.3 = 368; recent weekly pre-releases = 373–1108 each. A defect merged to `v1.0-dev` reaches weekly-build users within about 7 days.

**Threat model:**
- Users run a standard OS configuration: defaults changed only as far as normal use changes them. Findings that need a non-standard setup (hand-edited or crafted files in the profile directory, symlinked databases, unusual data-directory locations, broken permissions) are not in scope.
- In: secrets reaching logs, diagnostics, shell traces or committed files, test code included (AGENTS.md § Never commit secrets).
- In: what is signed differs from what the user confirmed (amount, recipient, fee, double send).
- Out: an attacker already running as the same OS user (reads the process environment or the X11 screen, plants files in the profile directory). Applies to the whole application, developer tooling included.
  - Exception: secrets stored on disk are in scope. Defects in how `platform-wallet` storage (dashpay/platform) protects them are filed upstream, not worked around in DET. DET's own choices stay in scope here: which secrets it stores protected or unprotected, and how imports and migrations pick that path.
- Out: hostile Platform data. DET trusts data from Platform and the SDK; validation and proofs are dashpay/platform's responsibility, and DET does not re-implement their checks.
- One interactive human per profile; concurrency is that human plus background tasks. Races that need unusual timing and lose neither funds nor local wallet or identity data are declined.

**Critical assets** (most important first):
1. Secret confidentiality: seed, private keys, passwords.
2. Funds.
3. What is signed equals what was shown (WYSIWYS).
4. Local wallet and identity data, and upgrade safety: migrations and the deletion chokepoint.
5. True, actionable user-facing text for the Everyday User (AGENTS.md § Error messages).
6. Client liveness: closing, network switch, refresh freshness.
7. Release integrity, and CI gates that test what they claim.
8. Robustness of developer and internal tooling and its docs.

**Accepted risks:**
- AR1 Hardening developer or internal tooling against its own operator or a same-user process.
- AR2 Non-standard OS or profile setup: hand-placed or crafted local files, database symlinks, oversized values in local databases, a transient SQLite `-shm` sidecar, a data directory outside the defaults.
- AR3 Devnet-only failures, e.g. a Devnet wipe racing an edit: a Devnet reset is a developer problem.
- AR4 Narrow races that lose neither funds nor local wallet or identity data.
- AR5 A form edited through assistive technology behind an open confirmation modal; the send uses the confirmed snapshot.
- AR6 Limitations owned by platform-wallet, the SDK or rust-dashcore: documented as a Known limitation with an upstream link, not worked around in DET.
- AR7 A full path in diagnostics the user chooses to share, when the data directory is outside the home directory (a non-standard setup); the home directory itself is redacted.
- AR8 Secrets stored without a password stay in the keyless vault until the user opts in to protection (AGENTS.md § wallet_backend).
- AR9 Behaviour identical to the base branch does not block a PR.

**Anchors** (maintainer outcome; band where a review rated it):
- A1 Blocking, fixed (WYSIWYS): the dispatched send could differ from the confirmation shown.
- A2 MEDIUM, fixed the same day in its own PR: Send stayed enabled while a token send was in flight, so a second click sent twice.
- A3 Blocking, fixed (secrets): raw settings-file content in shareable startup diagnostics; its pre-existing one-line twin was fixed in the same PR.
- A4 LOW, fixed even in dev tooling: a typed credential printed under inherited shell tracing.
- A5 Merged over as "total overkill": a typed credential visible in a dev script's child-process environment.
- A6 Fixed: UI text promising what the app does not do.
- A7 Fixed whenever raised: fixed-pattern keys or seeds in tests (project rule, AGENTS.md).
- A8 Declined: bot-blocking database symlink, crafted database, `-shm` sidecar.
- A9 LOW, declined twice: a Devnet wipe racing an identity edit.
- A10 Declined with a Known limitation and an upstream link: a multi-input shield passes the pre-flight, then fails on fee in the pinned platform-wallet.
