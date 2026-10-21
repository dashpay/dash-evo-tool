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
    local name="$1" phrase="$2" text=''
    shift 2
    rm -f "$scratch/payload"
    if ! env PATH="$scratch/bin:$PATH" PAYLOAD_FILE="$scratch/payload" GITHUB_REPOSITORY=owner/repo \
        GITHUB_RUN_ID=7 GITHUB_REF_NAME=v1.0-dev SLACK_WEBHOOK_URL=https://hooks.invalid/x \
        SHOULD_RELEASE=true CI_RED=false VERSION=1.0.0-weekly.20261013+abcdef012 \
        RELEASE_URL=https://example.invalid/release FINALIZE_RESULT=skipped PUBLISHED='' \
        RELEASE_RESULT=success FLATPAK_RESULT=success FIXTURE_RESULT=success \
        TAG_RESULT=success CLEANUP_RESULT=success "$@" \
        bash -euo pipefail "$scratch/notify.sh" > "$scratch/log" 2>&1; then
        text='<the notification step failed>'
    elif [[ -f "$scratch/payload" ]]; then
        text="$(jq -er .text "$scratch/payload")"
    fi
    if [[ -z "$phrase" && -z "$text" ]] || [[ -n "$phrase" && "$text" == *"$phrase"* ]]; then
        echo "PASS: $name"
    else
        failures=$((failures + 1))
        echo "FAIL: $name: expected '${phrase:-no message}', got: ${text:-no message}" >&2
    fi
}

outcome 'red CI' 'decline to bless a broken week' CI_RED=true SHOULD_RELEASE=false
outcome 'published' 'have published another weekly' FINALIZE_RESULT=success PUBLISHED=true \
    CLEANUP_RESULT=skipped
outcome 'draft sealed' 'have sealed another weekly draft' FINALIZE_RESULT=success \
    CLEANUP_RESULT=skipped
outcome 'build failed, cleaned up' 'the platform build faltered — I have swept' \
    RELEASE_RESULT=failure FIXTURE_RESULT=skipped
outcome 'build failed, cleanup failed' 'the platform build faltered, and automatic cleanup did not finish' \
    FLATPAK_RESULT=failure CLEANUP_RESULT=failure
outcome 'fixture capture failed, cleaned up' 'its migration fixture could not be captured — I have swept' \
    FIXTURE_RESULT=failure
outcome 'fixture capture failed, cleanup failed' \
    'its migration fixture could not be captured, and automatic cleanup did not finish' \
    FIXTURE_RESULT=failure CLEANUP_RESULT=failure
outcome 'build and fixture capture failed' 'the platform build faltered — I have swept' \
    FLATPAK_RESULT=failure FIXTURE_RESULT=failure
outcome 'tag failed' "could not even lay down this week's tag" TAG_RESULT=failure \
    RELEASE_RESULT=skipped FLATPAK_RESULT=skipped FIXTURE_RESULT=skipped CLEANUP_RESULT=skipped
outcome 'nothing to release' '' SHOULD_RELEASE=false
outcome 'no webhook' '' SLACK_WEBHOOK_URL=

# A called job that declares more than its caller grants fails the whole
# weekly at startup, before any job runs. `write` covers `read`.
permissions() { grep -E '^      [a-z-]+: (read|write)( |$)' | awk '{ print $1, $2 }' | sort -u; }
granted="$(sed -n '/^  fixture:$/,/^  finalize:$/p' "$workflows/weekly-build.yml" | permissions || true)"
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

if [[ "$failures" -ne 0 ]]; then
    echo "$failures weekly job check(s) failed." >&2
    exit 1
fi
