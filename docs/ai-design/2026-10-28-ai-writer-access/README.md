# AI writer access and automatic release hardening

Weekly publication remains automatic. PR Hygiene continues to own review policy;
its empty generated CODEOWNERS and native approval count are intentional.
The live access grant, protections and secret migration remain rollout work, not
changes already applied by this branch.

## Collect the missing administrator evidence

Run the collector from the operator's terminal, using the operator's existing
GitHub CLI login. The expected operator is `lklimek`; the agent is
`Claudius-Maginificent`. Environment tokens take precedence over stored CLI
credentials, so the commands unset both overrides for these reads only.

```bash
cd /data/git-worktrees/dash-evo-tool-ai-writer-hardening
env -u GH_TOKEN -u GITHUB_TOKEN gh api user --jq .login
env -u GH_TOKEN -u GITHUB_TOKEN python3 scripts/github/collect_access.py \
  --expected-user lklimek \
  --output /data/artifacts/dash-evo-tool/2026-10-28/ai-writer-hardening/admin-access.json
```

If the first command identifies another account, authenticate in the operator's
own CLI profile using `gh auth login --hostname github.com --web`, or select an
already configured operator account. Do not give that profile to the agent and
do not paste a token into chat. A separate Unix account or machine provides a
stronger credential boundary than a separate terminal under the same user.

The collector uses REST GET requests and read-only GraphQL queries. It records account/repository permissions,
organization membership, rulesets including bypass actors, Actions policies,
environment protections, secret **names and dates**, deploy-key metadata and
runner metadata. It also checks exact per-PR `isRequired` values, since the REST
ruleset response alone omitted the required PR Hygiene status during the audit.
It never requests secret values, prints tokens, changes settings or starts CI.

Output is created with mode `0600` and is never overwritten. Exit code 2 means
some reads were unavailable; the report still contains the successful reads and
the errors. A 403/404 is not evidence that a protection is disabled. The report
can be shared for follow-up; it contains infrastructure/account metadata but no
secret values. Use a new output filename when repeating collection.

## Changes prepared in this branch

- Required Tests and Clippy checks are emitted for documentation-only PRs too.
  A local Git-diff classifier skips unnecessary compilation, preserves fixture
  tooling coverage and falls back to execution when classification fails.
  Clippy runs directly with read-only permissions; its required `Clippy` job
  remains. The optional `Clippy Report` annotation from the retired action is
  removed.
- Weekly captures the event SHA once. Tag creation, platform builds and Flatpak
  builds use that same SHA. Feature-branch dispatch cannot enter the weekly gate.
- `release.yml` loads its source-authorization code from the explicit protected
  `refs/heads/v1.0-dev` branch, not from a candidate tag or caller-controlled input.
  Branch runs accept only `v1.0-dev` or `master`. Tagged commits must be ancestors
  of one of those branches. A tag input cannot redirect a branch run to different
  code, and publication checks the tag again against the authorized SHA.
- macOS build jobs produce unsigned app bundles with read-only repository access.
  Fresh signing jobs download bundles from the same workflow run, reject extra
  files and symlinks, and restore executable permissions before importing keys.
  Signing jobs do not check out the repository, execute the app, compile code or
  restore build caches.
- Only completed artifacts reach the job with publication and attestation
  authority. Existing release filenames and the `release.yml` attestation signer
  identity are preserved for migration-fixture verification.
- Flatpak publication has its own `flatpak-release.yml` caller, so a reusable
  build called by weekly needs only read permission. A skipped write-capable job
  inside the reusable workflow would still exceed a read-only caller's budget.
- Privileged signing/publication actions and the review action are pinned to
  immutable commits. Standard release-tag builds still do not automatically
  create a GitHub release unless the existing publication input is supplied.

## One-time rollout prerequisites

Prepare these settings before merging the workflow change, then grant the writer
access. The branch does not change live repository settings or move secrets.

1. Create the environment **`release-signing`**. Use no required reviewers and no
   wait timer: weekly must not stop for manual approval. Select explicit branch
   policies for `v1.0-dev` and `master`, plus the `v*` release-tag policy. Branch
   and tag policies are separate types. An environment only checks the workflow
   ref; it does not validate arbitrary code selected later by checkout.
2. Put `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`,
   `APPLE_TEAM_ID` and `APPLE_APP_SPECIFIC_PASSWORD` in that environment. Supply
   values from the operator's secure store; GitHub cannot reveal existing secret
   values for copying. Remove repository-level copies and any equivalent
   organization-secret grants to this repository after confirming the new
   entries. Retained global copies leave the old credential path open.
3. Protect release branches and tag creation/update/deletion before giving the
   agent write access. Decide the release automation's narrowly scoped bypass
   identity from the collected evidence. Granting bypass to all GitHub Actions
   workflows also authorizes their old/untrusted workflow versions. Do not
   silently substitute a PAT in the weekly tag job: `GITHUB_TOKEN` suppresses
   recursive tag-triggered release runs; a PAT would change that behavior.
4. Keep the agent out of ruleset bypass, owner/reviewer rosters and administrator
   credentials. `Claudius-Maginificent` is already a PR Hygiene machine author.
   Preserve the required PR Hygiene status; the successful controller job is a
   different check. Confirm organization membership before selecting a token:
   fine-grained PAT support differs for outside collaborators.
5. Use a token without Workflows write, Administration, Secrets or commit-status
   write for ordinary agent work. Actions write is needed for reruns/dispatch, so
   separately assess privileged workflow execution policies. The shared Actions
   status-writer identity is not a boundary against arbitrary write-capable
   workflows. Do not tie scheduled runs to an assumed permanent `github.actor`.
6. Before the next stable release, update `master` through the normal reviewed
   release process so it contains these workflow changes. Historical workflow
   runs retain their old workflow code; removing global signing credentials also
   removes their old signing path. Signing from arbitrary feature refs is
   intentionally rejected.

These are rollout prerequisites, not claims that the unread settings are absent.
The administrator report is needed to prepare exact rules without breaking
existing release automation or widening its bypass privileges.

## Verification

```bash
python3 -m unittest discover -s scripts/ci -p 'test_release_*.py' -v
python3 -m unittest discover -s scripts/ci -p 'test_pr_scope.py' -v
python3 -m unittest discover -s scripts/github -p 'test_*.py' -v
bash scripts/ci/test-weekly-gate.sh
bash scripts/ci/test-weekly-jobs.sh
actionlint .github/workflows/release.yml .github/workflows/weekly-build.yml \
  .github/workflows/flatpak.yml .github/workflows/flatpak-release.yml \
  .github/workflows/release-policy.yml .github/workflows/claude-code-review.yml \
  .github/workflows/tests.yml .github/workflows/clippy.yml
```

The workflow-contract tests use PyYAML 6.0.3; `release-policy.yml` installs the
parser before running the tests. Source-policy and collector tests use mocked
APIs or isolated local Git repositories. App validation tests execute only the
pre-signing structural check with dummy files. No test runs the app or signs it.
macOS compilation, codesigning and Apple notarization require an actual release
run after the settings/secrets rollout; they cannot be proved by the Linux tests.

## Primary documentation

- [Deployment environment protections and ref matching](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments)
- [Environment secrets in reusable workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows#using-inputs-and-secrets-in-a-reusable-workflow)
- [Actions execution policies](https://docs.github.com/en/actions/how-tos/administer/control-workflow-execution)
- [Personal access token limitations](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#fine-grained-personal-access-tokens-limitations)

<sub>Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
