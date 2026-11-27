# Release publication hardening for an AI writer (plan v2)

**Status: Final — not implemented; PR #1161 closed as overkill.**

This file records the plan prepared for PR #1161 ("isolate release signing and
constrain agent write paths"), including the maintainer's decisions on
2026-11-27. The maintainer closed the PR as more machinery than the risk
justifies. Nothing below is implemented; treat it as a reference if the
question returns. "confirm" marks a platform claim that was not verified.

## 1. Goal

Let an AI agent hold the repository Write role while keeping the published
release assets, and the release metadata that steers users (title, notes,
prerelease and Latest flags), trustworthy.

## 2. Maintainer decisions (MQ1–MQ8)

| ID | Decision | Effect on the plan |
|---|---|---|
| MQ1 = c | The agent gets Write **now**. The controls land while work is in progress, not as a precondition. Order the work so the highest-risk gaps close first. | The plan is ordered by risk: admin-only controls first, then the guard, then the publish path. |
| MQ2 | The credential is either a bot account that is a dashpay org member with a fine-grained PAT, or a new org-owned GitHub App (arranged by the maintainer). Design for both. Keep the post-grant 403 probes (dispatch, disable workflow, secret and variable writes, environment edits). | §4.2, §9 |
| MQ3 = c | Releases published before the change get an admin-held baseline plus the guard only. No pruning, no re-issue. | §6.4 |
| MQ4 = b | Manual publication stays a populated draft plus a human click in the UI. The guard verifies afterwards. No "publish verified run N" job. | The residual for manual drafts is accepted (§10). |
| MQ5 = b | On a provenance or asset mismatch the guard **alerts only**: a Slack message (webhook secret in an admin-managed environment the agent cannot write) and, where possible, a warning prepended to the release notes. **Never auto-delete.** | Also applies to the publisher's own post-publish check (§7.2 step 9). |
| MQ6 = b | Title, notes and flag edits produce a Slack alert only, with no restore. **Hard requirement: no edit loop.** The notes warning is added at most once per (release, finding), never re-added after a human removes it, and alerts are deduplicated. | §6.5 ledger |
| MQ7 = a | Non-weekly `release.yml` publication is gated by an environment with the maintainer as required reviewer and "Prevent self-review". Weeklies stay automatic. | Follow-up B3 |
| MQ8 | Delete only the stale branches that carry old privileged workflow versions, from a list the maintainer approves; delete nothing without that approval. Do not touch `master`'s workflows (the maintainer updates `master` himself). No new collector or CI check. | Admin step Z6. `master` remains a workflow source the agent could reuse; the attestation policy (§5) makes the result unacceptable. |
| Kept | Certificate-field attestation policy (§5) and the guard model: the `release` event is a fast path, a 15-minute scheduled sweep is the authoritative check, and the cutover is by release ID (§6). | §5, §6 |

Questions still open when the PR was closed are listed in §11.

## 3. What happened to the plan

- PR #1161 was closed as overkill. Its workflow, script and test changes do
  not land.
- The approved stale-branch cleanup (MQ8) is handled separately and does not
  depend on this plan.

What stays useful from this work:

- The threat model (§4) and the list of platform facts (§4.4) describe what
  the Write role and the Releases API actually allow.
- The attestation policy (§5) is the minimal check that matters: verify
  certificate fields (exact ref, build config, trigger, run), not only the
  signer workflow name.
- The admin steps Z1–Z4 (§9) are cheap settings changes that work without any
  of the code: a narrowly scoped credential with 403 probes, a baseline of
  existing releases, immutable releases, and a tag ruleset.
- The by-ID publish sequence (§7.2) and the no-edit-loop alert ledger (§6.5)
  are worked designs for when release automation is next touched.

## 4. Threat model

### 4.1 Asset and zone

**Asset**: the bytes users install from the release page, and the release
metadata that steers them. **Zone**: boundary. The app has no auto-updater.
The migration-fixture CI consumes published assets. At the time of writing
there were 54 published releases; immutable releases were not enabled, and
`releases/latest` was `v0.9.3`.

### 4.2 Actor and credential (MQ2)

The repository **Write role** grants more than Contents. It also covers
releases, "create, edit, run, re-run and cancel workflows", and Actions
secrets and variables write
([roles](https://docs.github.com/en/organizations/managing-user-access-to-your-organizations-repositories/managing-repository-roles/repository-roles-for-an-organization),
[use secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets)).
Custom roles need Enterprise Cloud, so the effective limit is the
**credential**:

| Variant | Mechanism | Permission set (both variants) | Must also hold |
|---|---|---|---|
| C1: bot org member with a fine-grained PAT | Org member with the repository Write role. Fine-grained PATs are allowed for members but not for outside collaborators ([PAT limits](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)). The PAT grants the intersection of its permissions and the role. | Contents RW, Pull requests RW, Issues RW, Metadata R, Actions **R**. **No** Workflows, Secrets, Variables, Environments, Administration, Commit statuses or Deployments. | The bot's web login (password and 2FA) stays with the admin; the agent never holds a session, because a session carries the full Write role. The org requires approval of fine-grained PATs. |
| C2: org-owned GitHub App | Installation token. No collaborator role is needed: the App's permissions are the whole grant. | Same set, as App permissions | The App private key exists only on the agent host. Commits show as `<app>[bot]`, so the PR Hygiene machine-author list must be updated (**confirm**). |

With either variant the agent keeps every Releases API call, can push
branches and tags, and may be able to commit a workflow file identical to one
on another branch. That "same-file exception" is documented for the OAuth
`workflow` scope
([scopes](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps));
for fine-grained PATs and Apps it needs a probe (§9). Without Actions write
the agent cannot dispatch, re-run or disable workflows, or delete artifacts.
Without Secrets or Variables write it cannot overwrite webhooks or `vars.*`.

### 4.3 Agent actions and planned controls

| ID | Action | Planned control |
|---|---|---|
| A1 | Changes the assets of a weekly draft before publication | The publisher checks the release by ID against the sealed manifest; assets reach the draft only minutes before publication. |
| A2 | Edits the draft's title, notes or `tag_name` | The publisher writes the canonical title and notes in the publishing `PATCH` and checks `tag_name` by ID. |
| A3 | Moves the tag before publication | Tag → commit check; tag ruleset (Z4); immutability after publication. |
| A4 | Replaces our draft with a look-alike | The publisher addresses the release by ID only. |
| A5 | Publishes a draft itself, including a manual draft waiting for a human click | Guard verification (alert plus notes warning). Residual accepted (MQ4). |
| A6 | Publishes its own release, including one built by running an old workflow version from its own branch or tag | The attestation policy rejects such attestations; tag ruleset; the guard alerts. |
| A7 | Edits title or notes, or flips prerelease/Latest, on any release | The guard alerts once per change (MQ6); no restore. Residual accepted. |
| A8 | Changes or deletes releases published before the change | The guard compares against the admin baseline and alerts. Residual accepted (MQ3, MQ5). |
| A9 | Pushes to a same-repo PR head before the maintainer applies the `release-qa` label | Follow-up B2. Until then, the label is not applied to such PRs. |
| A10 | Creates a `v*` tag so that a genuine but untested build is published | Tag ruleset; `release-publish` environment (MQ7). |

### 4.4 Platform facts

| ID | Fact | Source |
|---|---|---|
| F1 | Immutable releases still allow the prerelease and Latest flags to change. Assets "can't be added, modified, or deleted" after publication. Only releases published after enabling become immutable; drafts stay mutable. | [Immutable releases](https://docs.github.com/en/code-security/supply-chain-security/understanding-your-software-supply-chain/immutable-releases), [GA changelog](https://github.blog/changelog/2025-10-28-immutable-releases-are-now-generally-available/), [prevent changes](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes) |
| F2 | Only `signature.certificate` and `verifiedTimestamps` cannot be influenced by the attesting workflow; `statement.predicate`, including `invocationId`, can. `--signer-workflow` without `@ref` matches any ref. Use `--cert-identity <url>@<ref>` and assert the certificate extensions from `--format json`. | `gh attestation verify --help` (2.105.0); [manual](https://cli.github.com/manual/gh_attestation_verify) |
| F3 | For `release` events, `GITHUB_SHA` is the tag's commit, so the workflow file is read from there. Workflows are not triggered for `created`, `edited` or `deleted` on **draft** releases. A `workflow_dispatch` triggered by `GITHUB_TOKEN` does create a run. | [Events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows), [trigger a workflow](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow) |
| F4 | Release `created_at` is documented as "the date of the commit used for the release", so it is no cutover key. Release `id` is. | [Releases REST](https://docs.github.com/en/rest/releases/releases) |
| F5 | Only repository admins configure environments. Environment secrets reach only jobs that reference the environment and pass its branch or tag policy. "Prevent self-review" stops the user who triggered a run from approving it. "Allow administrators to bypass configured protection rules" exists and is probably on by default (**confirm**). | [Manage environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments), [environments reference](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments) |
| F6 | A weekly asset set totals about 525 MB; each Flatpak is about 35–40 MB. | Weekly 20261117 |

## 5. Attestation policy: `scripts/ci/release_attestation.py`

One module, shared by the publisher and the guard, and later by release QA
`prepare.py`, `migration-fixture-capture.yml` and the admin verification
recipe.

It runs `gh attestation verify <file> -R dashpay/dash-evo-tool
--deny-self-hosted-runners --format json` and asserts **certificate fields
only**:

| Asset class | SAN (`--cert-identity`) | BuildConfigURI | BuildTrigger | Source digest / ref | Extra |
|---|---|---|---|---|---|
| Weekly platform (5) | `https://github.com/dashpay/dash-evo-tool/.github/workflows/release.yml@refs/heads/v1.0-dev` | `…/weekly-build.yml@refs/heads/v1.0-dev` | `schedule` or `workflow_dispatch` | `--source-ref refs/heads/v1.0-dev`; `--source-digest` = `--signer-digest` = tag commit | RunInvocationURI run ID = the expected run (publish path), or one run for every asset of the release (guard) |
| Weekly Flatpak (2), attested in `seal` | `…/weekly-build.yml@refs/heads/v1.0-dev` | same | same | same | same run ID |
| Weekly fixture (2) | `…/migration-fixture-capture.yml@refs/heads/v1.0-dev` | `…/weekly-build.yml@refs/heads/v1.0-dev` | same | `--source-ref refs/heads/v1.0-dev` | same run ID |
| Stable platform (5) | `…/release.yml@refs/tags/<tag>` | `…/release.yml@refs/tags/<tag>` | `push` (plus the follow-up's reviewed publish path) | `--source-digest` = tag commit | The tag commit is an ancestor of `master` or `v1.0-dev` (`git merge-base --is-ancestor` on a fresh clone) |

For every class, the attestation subject name must equal the asset name, and
`sha256(file)` must equal the asset `digest`.

Effect: an old workflow version run from an agent branch or tag produces a
SAN ending `@refs/heads/<agent-branch>` or `@refs/tags/<agent-tag>`, with a
commit that is not an ancestor of a trusted branch, so the policy rejects it.
`master`'s workflow (left unchanged per MQ8) can still run; the policy makes
its output unacceptable but cannot stop it, so the guard alerts on any such
release.

## 6. Release guard (follow-up B1)

### 6.1 Triggers and trust

**Authoritative sweep**
- `schedule: "*/15 * * * *"` on the default branch, plus `workflow_dispatch`;
  `concurrency: release-guard` with no cancel.
- The job references environment `release-guard`, whose deployment policy
  allows only branch `v1.0-dev`.
- Its secrets (`SLACK_RELEASE_GUARD_WEBHOOK` and the ledger credential, §6.5)
  are written by admins (F5) and are unreachable from agent branches, PR
  merge refs and tags.

**Fast path**
- `on: release: [published, edited, deleted, prereleased, released]`.
- Job permissions: `actions: write` only; no environment, no checkout.
- Single step: `gh workflow run release-guard.yml --ref v1.0-dev`.
- It can be bypassed (a tag on a commit without the file, F3), so nothing
  relies on it.
- No `github.event.release.*` value reaches a `run:` block.

**Policy code**: the sweep checks out `refs/heads/v1.0-dev`, sparse to
`scripts/ci/`, with `persist-credentials: false`.

### 6.2 Scope and cutover

- Releases with `id` greater than `CUTOVER_RELEASE_ID` (the highest release
  ID at merge, a constant; F4) get full verification by class (§5).
- Releases at or below the cutover are compared against the admin baseline
  only (§6.4), because many lack full certificate provenance.
- Drafts are listed too. A finding on a draft is alerted but never written
  into its notes (MQ4: a human publishes it).

### 6.3 Checks per sweep

Verification results are cached by `(asset_id, digest)` in the ledger, so
only new or changed assets are downloaded. A no-op sweep takes about 30 s,
roughly 3k short runs a month.

**New published releases**
- The asset set equals the class set exactly.
- The policy (§5) passes for each asset, and all assets come from one run.
- Tag → commit equals the certificate source digest.
- `immutable == true` (enforced once immutability is enabled).
- Weeklies only: `prerelease == true`, not Latest, and title and notes equal
  the canonical text, or the canonical text plus the guard's own warning.

**Earlier releases**: compare `{asset_id, name, size, digest}`, `prerelease`,
Latest, tag → commit, and the `sha256` of the title and the notes against the
baseline.

**Release set**: report deleted releases, and new releases on tags the tag
rulesets should have blocked.

### 6.4 Baseline of earlier releases (MQ3)

- Before the agent uses the grant, the admin captures the baseline with their
  own credentials: `gh api --paginate repos/dashpay/dash-evo-tool/releases`,
  reduced to `{release_id, tag, tag_commit, prerelease, latest,
  title_sha256, notes_sha256, assets[{id, name, size, digest}]}`.
- The admin commits it to the ledger repository (§6.5) as `baseline.json`.
  The guard reads it and never writes it.
- A deliberate maintainer change to an earlier release is recorded as a new
  baseline commit by the admin.

### 6.5 Alert and notes-warning ledger (MQ5, MQ6: no edit loop)

**Store**
- A private repository owned by an org admin. The agent's credential is not
  installed there and is not a collaborator.
- The sweep writes to it with a **write deploy key**, held as a secret of the
  `release-guard` environment. An org-owned App with Contents write on that
  repository only would work equally.
- The state is durable, unreachable for the agent, and audited by git
  history.

**Rejected stores**
- A hidden marker in the notes: the agent or a human can remove it.
- Actions cache: evicted after 7 days unused, and writable by any job scoped
  to `v1.0-dev`.
- A branch in this repository: a ruleset bypass for Actions covers every
  workflow version.
- Environment variables: writing them needs an admin-capable token inside the
  guard.

**Fallback** (no second repository): a rolling artifact chain. Each sweep
downloads the ledger artifact of the last successful sweep and re-uploads it
(90-day retention, renewed every 15 minutes). If the chain is lost, the guard
fails closed (below).

**Records**: `ledger/<release_id>.json`, append-only.
- `finding_key = sha256(release_id, kind, subject, observed_value)`, e.g.
  `kind=asset_digest, subject=<asset name>, observed=sha256:…` or
  `kind=notes, observed=sha256(notes)`.
- `alerted_at`, `notes_warning_added_at`, and `self_written_notes_sha256[]`.

**Algorithm**, per finding, in order:
1. If `finding_key` is in the ledger, **do nothing**: no alert, no edit. A
   warning removed from the notes changes nothing here, because this decision
   never reads the notes.
2. Commit and push `{finding_key, alerted_at: now}` to the ledger. If the push
   fails, skip the finding in this sweep: no alert, no edit.
3. Send the Slack alert. If sending fails, the record stays and only the send
   is retried in the next sweep (`alert_pending: true`).
4. Only for a provenance or asset mismatch on a **published** release, and
   only once:
   - commit `notes_warning_intended`;
   - `PATCH` the notes to `WARNING: <fixed text naming the asset> — do not
     install; maintainers notified.` followed by the current notes;
   - record `notes_warning_added_at` and add `sha256(new notes)` to
     `self_written_notes_sha256`.

   If the `PATCH` fails after `intended` was committed, it is **not** retried
   (at most once); the alert has already gone out.
5. A notes change whose hash is in `self_written_notes_sha256` is not a
   finding, so the guard's own edit never re-triggers it. Any other notes
   change becomes a new `kind=notes` finding: one alert per distinct content,
   never an edit.

**Ledger unavailable** (clone or push fails): no notes edits at all, and at
most one "ledger unavailable" alert per UTC hour, sent only from the sweep
scheduled at minute `:00`, which deduplicates by time without state.

## 7. Weekly publish path (the #1161 core)

### 7.1 Flow

| Job | Change |
|---|---|
| `release` (`release.yml`, weekly) | Builds, checks and attests. **Creates no release**: the weekly `softprops` step is removed. |
| `fixture` | `attach: false`. The fixture stays a run artifact. |
| `seal` | No release access: `contents: read`, plus `id-token`/`attestations: write` for the Flatpak attestation only. Downloads the 9 run artifacts, attests the 2 Flatpaks, verifies all 9 with the policy against `$GITHUB_RUN_ID`, and outputs `manifest` `{name: sha256}`, frozen before QA. |
| `qa` | `contents: read`. Tests the binary from this run. `prepare.py` no longer reads any release. |
| `finalize` | Runs `release_publish.py`: a pre-create scan by tag, then everything by **ID** (§7.2). Mode is `publish` for a scheduled run with QA PASS, otherwise `draft-only`. |
| `cleanup` | Build or fixture failures happen before any release exists, so it deletes only the tag this run created (after `verify_remote_tag`). It never deletes a release; if one references the tag, it alerts. |

A draft then holds assets only for the upload time (about 525 MB, minutes)
instead of hours. Integrity rests on the by-ID checks and the policy; the
short window also removes write access from `seal` and `qa`, and lets
`release-qa.yml` `prepare` drop to `contents: read` (a nested job cannot
exceed its caller's permissions, so both must change together).

### 7.2 Publish sequence (`scripts/ci/release_publish.py`)

1. `verify_remote_tag`: the tag points at the authorized commit.
2. Re-download the run artifacts. Require their `sha256` to equal the sealed
   manifest, with exactly the same set of names.
3. Pre-create scan of releases with `tag_name == TAG`:
   - none: continue;
   - exactly one marked draft from this workflow: re-`GET` it by ID, re-check
     draft state and marker, then `DELETE` it by ID;
   - anything else: refuse and alert.
4. `POST /releases` with `{draft: true, prerelease: true, make_latest:
   "false", tag_name, target_commitish: SHA, name, body}` (canonical) →
   **`id`**.
5. Upload each file by `id`. A 422 means a foreign asset: refuse and alert.
   The draft is retained; nothing is deleted after this point (MQ5).
6. Pre-publish `GET /releases/{id}` must show `draft`, `tag_name` and
   `prerelease` as expected, and exactly the expected asset set, each
   `digest` equal to the manifest and `state == uploaded`. In `draft-only`
   mode the sequence stops here and outputs `release_id` and `url`.
7. `PATCH /releases/{id}` with `draft: false`, `prerelease: true`,
   `make_latest: "false"`, and the canonical title and notes.
8. Post-publish checks: `GET /releases/{id}` (`draft == false`,
   `immutable == true`, same set and digests); `GET /releases/tags/{TAG}`
   returns `id`; `gh release verify TAG --format json` lists exactly the
   manifest digests; tag → commit. A "not yet available" result is retried
   with backoff for up to about 5 minutes.
9. On a mismatch or a timeout, **alert only (MQ5)**: add the at-most-once
   notes warning under the ledger rules (§6.5) if the ledger secret is
   available, otherwise the next guard sweep adds it; fail the job; **never
   delete**.

Steps 6 and 7 cannot be atomic (there is no conditional write). Immutability
freezes the release at step 7, so step 8 sees exactly what users receive. A
detected swap stays public until a human acts; this residual is accepted.

After step 4, every read and mutation uses `/releases/{id}`; calls by tag
remain only as cross-checks. Manual and non-PASS runs (MQ4) end with a
populated draft whose `release_id` goes into the Slack message; the human
publishes it, and the next sweep verifies it. The maintainer can check the
draft first with `scripts/ci/release_guard.py --check <release_id>`
(optional, read-only).

### 7.3 Non-weekly `release.yml` (follow-up B3)

The tag-push path created a published release and then uploaded assets to it,
which fails once immutability is on. The follow-up moves it to the same
sequence behind a `release-publish` environment (MQ7), builds and attaches
the Flatpaks before publication, and removes the `release: published` trigger
and the attach-after-publish step in `flatpak-release.yml`. Until then, no
stable or tag release can be cut with immutability enabled.

## 8. Delivery plan (ordered by risk)

LOC = added plus deleted code and test lines, excluding docs; estimates.
"Before" = in place before the first sensitive use (any agent call to the
Releases or tag API, agent tag push, agent push to a PR that gets the
`release-qa` label, or agent work on `.github/**`).

| # | Item | Kind | Size | Gate |
|---|---|---|---|---|
| 1 | Z1 credential + 403 probes | admin | 30 min | before (at grant) |
| 2 | Z2 baseline of earlier releases | admin | 10 min | before |
| 3 | Z3 enable immutable releases | admin | 5 min | before |
| 4 | Z4 all-tags ruleset + probe | admin | 30 min | before |
| 5 | Z5 no `release-qa` label on agent-pushable PRs until B2; Z7 no tag or stable releases until B3; Z8 the agent never calls the Releases or tag API and never pushes tags | operational | none | before |
| 6 | Z6 delete the approved stale branches (MQ8) | admin | 15 min | before |
| 7 | B1 release guard + ledger | PR | ~730–900 LOC | first code item |
| 8 | #1161 scope A1–A6 | PR | ~1,420 LOC | next |
| 9 | B5 no Cargo cache restore in release builds | PR | ~25 LOC | before the next weekly if possible |
| 10 | B2 labelled release QA from base-branch scripts | PR | ~90 LOC | before lifting Z5 |
| 11 | B3 draft-first stable path + `release-publish` environment | PR | ~250 LOC | before the next stable or tag release |
| 12 | B4 fixture capture verified with the policy | PR | ~60 LOC | after |
| 13 | B6 migration password only for trusted-ref runs | PR | ~25 LOC | after |

### 8.1 #1161 scope

| ID | Item | Content | LOC |
|---|---|---|---|
| A1 | Attestation policy | `scripts/ci/release_attestation.py` (§5), tests AT1–AT7 | ~350 |
| A2 | By-ID weekly publish | `scripts/ci/release_publish.py` (§7.2) and the `weekly-build.yml` / `release.yml` changes of §7.1; tests UT1–UT13 and a `test-weekly-jobs.sh` scenario | ~700 |
| A3 | Release QA hardening | `release-qa.yml`: pin the three `download-artifact` uses to `37930b1c… # v7.0.0`; `prepare` → `contents: read`; `source_sha` input. `prepare.py`: no release lookup when the binary and `NEW_SHA` are present; policy via A1; refuse an unpublished tag without `run_id` | ~90 |
| A4 | Action pins | `migration-fixture-capture.yml`: checkout, upload-artifact and attest pinned to SHAs; `release.yml`: pin `actions/cache`, drop `dtolnay/rust-toolchain@stable` ×2 in favour of `rust-toolchain.toml`; `flatpak.yml`: drop or pin `dtolnay` | ~12 |
| A5 | Policy tests + filters | PT1–PT9 (§8.3); `release-policy.yml` paths add `migration-fixture-capture.yml`, `.github/actions/**` and `scripts/migration-fixtures/**` | ~230 |
| A6 | Docs | `2026-10-28-ai-writer-access/README.md` item 6: the controls and accepted residuals that actually apply; PR body | docs |

### 8.2 Follow-up PRs

- **B1 `feat(ci): verify published releases with a scheduled release guard`**:
  `release-guard.yml` (~80), `release_guard.py` (~300: class checks, baseline
  diff, ledger, Slack, at-most-once warning, `--check`), tests (~350, GT1–GT13
  below plus policy tests: no `github.event.release` in `run:`, only the
  sweep uses the environment, the fast path has no checkout and only
  `actions: write`, the guard file is pinned). Admin: `release-guard`
  environment, secrets, ledger repository and deploy key, `baseline.json`,
  `CUTOVER_RELEASE_ID`, one dispatch dry run.
- **B2 `fix(ci): run labelled release QA from base-branch scripts`**: check
  out `scripts/ci/release-qa/**` from `pull_request.base.sha`; only the build
  input comes from the head; move the label allow-list from a variable into
  the workflow. ~90 LOC.
- **B3 `fix(release): publish tag releases draft-first behind a reviewed
  environment`**: §7.3; admin creates `release-publish` with the maintainer as
  required reviewer, "Prevent self-review" and tag policy `v*`. ~250 LOC.
- **B4 `fix(ci): verify fixture-capture binaries with the release attestation
  policy`**: `migration-fixture-capture.yml` and `download-fixtures.sh` use
  the policy; `attach` additionally requires `refs/heads/v1.0-dev`. ~60 LOC.
- **B5 `fix(ci): stop restoring the Cargo cache in release builds`**: remove
  the `actions/cache` step from `release.yml` and assess Flatpak's cache
  steps; a policy test. ~25 LOC.
- **B6 `fix(ci): pass the v0.9.3 migration password only to trusted-ref
  runs`**: `tests.yml` plus a policy test. ~25 LOC.

### 8.3 Planned tests

Policy tests (`scripts/ci/test_release_workflow.py`):

| ID | Asserts |
|---|---|
| PT1 | In the release workflows, every `uses:` is local or `owner/repo[/path]@<40-hex>` with a `# v…` comment. `claude-code-review.yml` is excluded. |
| PT2 | Every `actions/download-artifact@` ref in `.github/workflows/*.yml` equals the approved SHA. |
| PT3 | Every checkout in those files sets `persist-credentials: false`, except `weekly-build.yml` `jobs.tag`. |
| PT4 | No `gh release (create\|edit\|upload\|delete)` and no `softprops/` on the weekly path; `finalize` runs the publisher; `cleanup` deletes only `git/refs/tags`. |
| PT5 | `contents: write` only in weekly `tag`, `finalize`, `cleanup` and the `release` call; none in `seal`, `qa` or `release-qa.yml`. |
| PT6 | `qa` needs `seal`; `finalize` needs `seal` and `qa`; the manifest comes from `seal`. |
| PT7 | The `finalize` mode expression. |
| PT8 | `fixture.with.attach` is `false`. |
| PT9 | The policy path filters. |

Unit tests: AT1–AT7 for the policy (SAN on another ref, wrong BuildConfigURI
or trigger, run mismatch with a forged predicate, subject name mismatch,
stable tag not an ancestor, self-hosted runner, a genuine certificate);
UT1–UT13 for the publisher (asset set or digest mismatch, moved tag, foreign
releases on the tag, a marked leftover draft, 422 on upload, post-publish
mismatch without `DELETE`, delayed attestation, concurrent publication, tag
lookup returning another ID, altered draft text, manifest mismatch, by-ID
call log, `immutable == false`); GT1–GT13 for the guard (one alert and one
record per finding, a removed warning is never re-added, the guard's own edit
is no finding, one alert per distinct notes hash, no alert or edit when the
ledger push fails, hourly "ledger unavailable" alert only, no retry of a
failed `PATCH`, baseline diffs, cutover by ID, drafts alert only, the asset
cache, an asset with a foreign SAN, Slack retry without a repeated edit).

## 9. Admin rollout

1. **Credential (MQ2)**: C1 or C2 per §4.2.
   - Probes with the agent credential, each expecting 403: `POST
     …/actions/workflows/{id}/dispatches`, `PUT
     …/actions/workflows/{id}/disable`, `PUT …/actions/secrets/X`, `POST
     …/actions/variables` and `PATCH …/actions/variables/X`, `PUT
     …/environments/X`, `POST …/actions/runs/{id}/rerun`.
   - Same-file probe: push a branch carrying an identical copy of `master`'s
     `release.yml`; record whether it is accepted. Nothing triggers on a
     branch push.
   - Positive probes: push a branch; open a PR.
2. **Immediately**: capture the baseline (§6.4); enable immutable releases;
   add a ruleset for all tags not yet covered (restrict creation, update and
   deletion; bypass for repository admins only) and probe it from the agent
   credential with both `POST /releases` on a new tag and `POST /git/refs`;
   apply no `release-qa` label to agent-pushable PRs until B2; delete the
   approved stale branches (MQ8).
3. **With B1**: `release-guard` environment (branch policy `v1.0-dev` only;
   secrets `SLACK_RELEASE_GUARD_WEBHOOK` and `RELEASE_LEDGER_DEPLOY_KEY`);
   private ledger repository with a write deploy key; commit `baseline.json`.
4. **With B3**: `release-publish` environment with the maintainer as required
   reviewer and "Prevent self-review" on.
5. Drop the earlier precondition that all admin steps precede the grant
   (MQ1 = c).

## 10. Accepted residuals

- **MQ3**: releases published before the change remain changeable; a change
  is only alerted on (with a notes warning) and stays until a human acts.
- **MQ4**: a manual draft can be changed before the human clicks publish. The
  guard verifies afterwards; `release_guard.py --check <id>` is available
  before the click.
- **MQ5**: a detected swap on a new release stays published (and immutable)
  until a human acts. If the human deletes the release and tag, that tag name
  cannot be reused.
- **MQ6**: title, notes, prerelease and Latest changes persist until a human
  acts.
- **Detection latency**: up to 15 minutes plus schedule delay; the event fast
  path can be bypassed.
- **MQ8**: `master`'s workflows stay as they are. The policy rejects their
  output for weekly and stable classes, and the guard alerts. There is no
  watchdog for a disabled guard; disabling it needs Actions write, which Z1
  withholds.

## 11. Open questions when the PR was closed

| ID | Question | Recommendation at the time |
|---|---|---|
| D1 | Ledger store: private ledger repository written through a deploy key in the admin-only `release-guard` environment, or a rolling artifact chain? | Ledger repository. |
| D2 | Keep A1–A2 (~1,050 LOC) in #1161, or carve them into a separate PR that merges right after it? | Carve out. |
| D3 | Does "never auto-delete" (MQ5) also cover the publisher's own post-publish mismatch? | Yes: alert, add the one-time warning, fail the job. |
| D4 | `release-publish` with "Prevent self-review": the maintainer, as sole reviewer, cannot approve runs he triggers himself. | Keep "Prevent self-review" on and rely on "Allow administrators to bypass"; add a second reviewer if one is available. |
| D5 | Enable immutable releases before any of the code lands? Weeklies keep working; stable and tag releases wait for B3, and fixture back-fill to published releases ends. | Yes, now. |

## Sources

- [Repository roles](https://docs.github.com/en/organizations/managing-user-access-to-your-organizations-repositories/managing-repository-roles/repository-roles-for-an-organization)
- [Using secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets)
- [OAuth scopes (`workflow` same-file exception)](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps)
- [Immutable releases](https://docs.github.com/en/code-security/supply-chain-security/understanding-your-software-supply-chain/immutable-releases), [GA changelog](https://github.blog/changelog/2025-10-28-immutable-releases-are-now-generally-available/), [prevent release changes](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes)
- [Releases REST](https://docs.github.com/en/rest/releases/releases), [release assets REST](https://docs.github.com/en/rest/releases/assets), [asset digests changelog](https://github.blog/changelog/2025-06-03-releases-now-expose-digests-for-release-assets/)
- [Permissions for fine-grained PATs](https://docs.github.com/en/rest/authentication/permissions-required-for-fine-grained-personal-access-tokens), [for GitHub Apps](https://docs.github.com/en/rest/authentication/permissions-required-for-github-apps), [PAT limitations](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
- [Rulesets: available rules](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets)
- [Manage environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments), [deployments and environments](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments)
- [Events that trigger workflows](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows), [trigger a workflow](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)
- [gh attestation verify](https://cli.github.com/manual/gh_attestation_verify)
