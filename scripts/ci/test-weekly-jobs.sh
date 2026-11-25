#!/usr/bin/env bash
# Exercise the weekly workflow around its build: every outcome the Slack
# notification reports, and the permissions its fixture job hands down.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workflows="$script_dir/../../.github/workflows"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/weekly-jobs-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin"
sed -n '/^          if \[ -z "..SLACK_WEBHOOK_URL:-}" \]; then$/,/^          echo "Slack notification sent."$/p' \
    "$workflows/weekly-build.yml" > "$scratch/notify.sh"
test -s "$scratch/notify.sh"
# Records the request body instead of posting it.
cat > "$scratch/bin/curl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
while (($#)); do
    [[ "$1" != -d ]] || printf '%s' "$2" > "$PAYLOAD_FILE"
    shift
done
MOCK
chmod +x "$scratch/bin/curl"

failures=0
# outcome NAME EXPECTED_PHRASE [VARIABLE=VALUE...]; an empty phrase expects no message.
outcome() {
    local name="$1" phrase="$2" text='' forbidden='' arg
    shift 2
    for arg in "$@"; do
        [[ "$arg" != FORBIDDEN_PHRASE=* ]] || forbidden="${arg#*=}"
    done
    rm -f "$scratch/payload"
    if ! env PATH="$scratch/bin:$PATH" PAYLOAD_FILE="$scratch/payload" GITHUB_REPOSITORY=owner/repo \
        GITHUB_RUN_ID=7 GITHUB_REF_NAME=v1.0-dev SLACK_WEBHOOK_URL=https://hooks.invalid/x \
        SHOULD_RELEASE=true CI_RED=false VERSION=1.0.0-weekly.20261013+abcdef012 \
        RELEASE_URL=https://example.invalid/release FINALIZE_RESULT=skipped PUBLISHED='' \
        RELEASE_RESULT=success FLATPAK_RESULT=success FIXTURE_RESULT=success \
        TAG_RESULT=success CLEANUP_RESULT=success QA_VERDICT='' QA_URL='' EVENT_NAME=schedule "$@" \
        bash -euo pipefail "$scratch/notify.sh" > "$scratch/log" 2>&1; then
        text='<the notification step failed>'
    elif [[ -f "$scratch/payload" ]]; then
        text="$(jq -er .text "$scratch/payload")"
    fi
    if [[ -n "$forbidden" && "$text" == *"$forbidden"* ]]; then
        failures=$((failures + 1))
        echo "FAIL: $name: unexpected phrase: $forbidden" >&2
    elif [[ -z "$phrase" && -z "$text" ]] || [[ -n "$phrase" && "$text" == *"$phrase"* ]]; then
        echo "PASS: $name"
    else
        failures=$((failures + 1))
        echo "FAIL: $name: expected '${phrase:-no message}', got: ${text:-no message}" >&2
    fi
}

outcome 'red CI' 'decline to bless a broken week' CI_RED=true SHOULD_RELEASE=false
outcome 'published' 'have published another weekly' FINALIZE_RESULT=success PUBLISHED=true \
    CLEANUP_RESULT=skipped
outcome 'missing QA outputs still show INCOMPLETE' 'GUI QA: INCOMPLETE.' FINALIZE_RESULT=success CLEANUP_RESULT=skipped
outcome 'missing QA report links workflow run' '<https://github.com/owner/repo/actions/runs/7|the workflow run>' FINALIZE_RESULT=success CLEANUP_RESULT=skipped
outcome 'draft sealed'  'have sealed another weekly draft' FINALIZE_RESULT=success \
    CLEANUP_RESULT=skipped
outcome 'build failed, cleaned up' 'the platform build faltered — I have swept the newly created tag and its broken draft away' \
    RELEASE_RESULT=failure FIXTURE_RESULT=skipped
outcome 'build failed, cleanup failed' 'the platform build faltered, and automatic cleanup did not finish' \
    FLATPAK_RESULT=failure CLEANUP_RESULT=failure
outcome 'fixture capture failed, cleaned up' 'its migration fixture could not be captured — I have removed the newly created tag and its broken draft' \
    FIXTURE_RESULT=failure
outcome 'fixture capture failed, cleanup failed' \
    'its migration fixture could not be captured, and automatic cleanup did not finish' \
    FIXTURE_RESULT=failure CLEANUP_RESULT=failure
outcome 'build and fixture capture failed' 'the platform build faltered — I have swept the newly created tag and its broken draft away' \
    FLATPAK_RESULT=failure FIXTURE_RESULT=failure
outcome 'tag failed' "could not even lay down this week's tag" TAG_RESULT=failure \
    RELEASE_RESULT=skipped FLATPAK_RESULT=skipped FIXTURE_RESULT=skipped CLEANUP_RESULT=skipped
outcome 'finalization failed' 'finalization did not complete. The draft is retained' \
    FINALIZE_RESULT=failure CLEANUP_RESULT=skipped QA_VERDICT=FAIL
outcome 'QA suffix for scheduled PASS' 'Scheduled runs publish only on PASS.' \
    FINALIZE_RESULT=success CLEANUP_RESULT=skipped QA_VERDICT=PASS PUBLISHED=true
outcome 'QA suffix for manual PASS' 'Manual runs remain drafts, including PASS.' \
    FINALIZE_RESULT=success CLEANUP_RESULT=skipped QA_VERDICT=PASS EVENT_NAME=workflow_dispatch
outcome 'QA suffix for failed QA' 'GUI QA: FAIL.' \
    FINALIZE_RESULT=success CLEANUP_RESULT=skipped QA_VERDICT=FAIL
outcome 'deleted draft has no QA suffix' 'I have swept' \
    RELEASE_RESULT=failure FIXTURE_RESULT=skipped QA_VERDICT=INCOMPLETE FORBIDDEN_PHRASE='GUI QA:'
outcome 'missing QA after failed finalization' 'GUI QA: INCOMPLETE.' \
    FINALIZE_RESULT=failure CLEANUP_RESULT=skipped
outcome 'nothing to release'  '' SHOULD_RELEASE=false
outcome 'PASS but publish failed' 'QA passed, but publishing failed' \
    FINALIZE_RESULT=success CLEANUP_RESULT=skipped QA_VERDICT=PASS
outcome 're-cut build failed' 'existing draft was retained and marked DO NOT PUBLISH' \
    RELEASE_RESULT=failure CLEANUP_RESULT=skipped FORBIDDEN_PHRASE='cleanup did not finish'
outcome 're-cut fixture failed' 'existing draft was retained and marked DO NOT PUBLISH' \
    FIXTURE_RESULT=failure CLEANUP_RESULT=skipped FORBIDDEN_PHRASE='cleanup did not finish'
outcome 'no webhook' '' SLACK_WEBHOOK_URL=

# A called job that declares more than its caller grants fails the whole
# weekly at startup, before any job runs. `write` covers `read`.
permissions() { grep -E '^      [a-z-]+: (read|write)( |$)' | awk '{ print $1, $2 }' | sort -u; }
granted="$(sed -n '/^  fixture:$/,/^  qa:$/p' "$workflows/weekly-build.yml" | permissions || true)"
declared="$(permissions < "$workflows/migration-fixture-capture.yml" || true)"
missing=''
while read -r scope level; do
    grep -qxE "$scope (write|$level)" <<<"$granted" || missing+=" $scope$level"
done <<<"$declared"
if [[ -n "$granted" && -n "$declared" && -z "$missing" ]]; then
    echo "PASS: the fixture job grants what the capture workflow declares"
else
    failures=$((failures + 1))
    echo "FAIL: the fixture job does not grant:${missing:- (no permissions found)}" >&2
fi

granted="$(sed -n '/^  qa:$/,/^  seal:$/p' "$workflows/weekly-build.yml" | permissions || true)"
declared="$(permissions < "$workflows/release-qa.yml" || true)"
missing=''
while read -r scope level; do
    grep -qxE "$scope (write|$level)" <<<"$granted" || missing+=" $scope$level"
done <<<"$declared"
if [[ -n "$granted" && -n "$declared" && -z "$missing" ]]; then
    echo "PASS: the qa job grants what the release QA workflow declares"
else
    failures=$((failures + 1))
    echo "FAIL: the qa job does not grant:${missing:- (no permissions found)}" >&2
fi

# Execute the first finalize step with a read-only API stub; no release edits.
sed -n '/^      - name: Verify reviewed tag has not moved$/,/^      - name: Record QA outcome/p' \
    "$workflows/weekly-build.yml" | sed -n '/^          set -euo pipefail$/,/^          fi$/p' > "$scratch/tag-check.sh"
test -s "$scratch/tag-check.sh"
for actual in reviewed replaced wrong-endpoint; do
    tag=v1.0.0
    [[ "$actual" != wrong-endpoint ]] || tag=unexpected
    if ACTUAL_SHA="${actual/wrong-endpoint/reviewed}" EXPECTED_SHA=reviewed GITHUB_REPOSITORY=owner/repo TAG="$tag" \
        bash -euc 'gh() { [[ "$*" == "api repos/owner/repo/commits/v1.0.0 --jq .sha" ]] || return 1; echo "$ACTUAL_SHA"; }; source "$1"' _ "$scratch/tag-check.sh" > "$scratch/log" 2>&1; then
        result=accepted
    else
        result=refused
    fi
    if [[ "$actual:$result" == reviewed:accepted || "$actual:$result" == replaced:refused || "$actual:$result" == wrong-endpoint:refused ]]; then
        echo "PASS: finalize tag binding ($actual)"
    else
        failures=$((failures + 1))
        echo "FAIL: finalize tag binding ($actual): $result" >&2
    fi
done

if [[ "$failures" -ne 0 ]]; then
    echo "$failures weekly job check(s) failed." >&2
    exit 1
fi
