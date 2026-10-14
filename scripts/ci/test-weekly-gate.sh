#!/usr/bin/env bash
# Exercise the workflow's release gate against offline CI run histories.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
workflows="$script_dir/../../.github/workflows"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/weekly-gate-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin"
sed -n '/^          red=false$/,/^          echo "ci_red=false"/p' \
    "$workflows/weekly-build.yml" > "$scratch/gate.sh"
test -s "$scratch/gate.sh"

# `gh run list` returns the newest matching run first; `gh run view` its jobs.
cat > "$scratch/bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
command="$1 $2"
shift 2
run_id=''
if [[ "$command" == 'run view' ]]; then
    run_id="$1"
    shift
fi
workflow='' event='' branch='' status='' limit='' query='' commit='' fields=''
while (($#)); do
    case "$1" in
        --workflow) workflow="$2" ;;
        --event) event="$2" ;;
        --branch) branch="$2" ;;
        --commit)
            [[ "$2" =~ ^[0-9a-f]{40}$ ]]
            commit="$2"
            ;;
        --status) status="$2" ;;
        --limit) limit="$2" ;;
        --json) fields="$2" ;;
        --jq) query="$2" ;;
        *) exit 2 ;;
    esac
    shift 2
done
[[ -n "$fields" && -n "$query" ]]
# GH_FAIL names the query that fails: a command, or the event a listing filters on.
[[ "${GH_FAIL:-}" != "$command" && "${GH_FAIL:-}" != "${event:-none}" ]] || exit 1
case "$command" in
    'run list')
        [[ -n "$workflow" && -n "$branch" && "$status" == completed && "$limit" == 1 ]]
        jq --arg workflow "$workflow" --arg event "$event" --arg branch "$branch" --arg commit "$commit" '
            [.[] | select(.workflow == $workflow and .headBranch == $branch
                          and ($event == "" or .event == $event)
                          and ($commit == "" or .headSha == $commit))]
            | sort_by(.createdAt) | reverse | .[:1]' "$RUN_HISTORY" | jq -r "$query"
        ;;
    'run view')
        [[ "$fields" == jobs ]]
        jq --argjson id "$run_id" '.[] | select(.databaseId == $id) | {jobs}' "$RUN_HISTORY" |
            jq -r "$query"
        ;;
    *) exit 2 ;;
esac
MOCK
cat > "$scratch/bin/git" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == 'rev-parse HEAD' ]]
printf '%s\n' "$HEAD_SHA"
MOCK
chmod +x "$scratch/bin/gh" "$scratch/bin/git"

head_sha=1111111111111111111111111111111111111111
other_sha=2222222222222222222222222222222222222222
oldest=2026-10-12T04:00:00Z
older=2026-10-12T05:00:00Z
newer=2026-10-13T06:00:00Z
newest=2026-10-13T07:00:00Z
reds=(failure cancelled timed_out startup_failure)
# The jobs a manual run of a release branch contains; variants drop one check.
all_checks='[{"name":"Platform pin guard","conclusion":"success"},
             {"name":"Test Suite","conclusion":"success"},
             {"name":"Fixture coverage","conclusion":"success"}]'
coverage_skipped="$(jq -c 'map(select(.name == "Fixture coverage").conclusion = "skipped")' <<<"$all_checks")"
pin_guard_absent="$(jq -c 'map(select(.name != "Platform pin guard"))' <<<"$all_checks")"

# run WORKFLOW EVENT CONCLUSION CREATED_AT [HEAD_SHA] [BRANCH] [JOBS]
run() {
    jq -cn --arg workflow "$1" --arg event "$2" --arg conclusion "$3" --arg createdAt "$4" \
        --arg headSha "${5:-$head_sha}" --arg headBranch "${6:-v1.0-dev}" \
        --argjson jobs "${7:-$all_checks}" '$ARGS.named'
}
green_pushes=("$(run tests.yml push success "$older")" "$(run clippy.yml push success "$older")")

failures=0
# scenario NAME EXPECTED_CI_RED RUN...
scenario() {
    local name="$1" expected="$2" status=0 problem=''
    shift 2
    printf '%s\n' "$@" | jq -s 'to_entries | map(.value + {databaseId: (.key + 1)})' \
        > "$scratch/history.json"
    : > "$scratch/output"
    env PATH="$scratch/bin:$PATH" RUN_HISTORY="$scratch/history.json" HEAD_SHA="$head_sha" \
        GH_FAIL="${GH_FAIL:-}" REF_NAME=v1.0-dev FORCE=true MANUAL_VERSION=1.0.0 \
        GITHUB_OUTPUT="$scratch/output" bash -euo pipefail "$scratch/gate.sh" \
        > "$scratch/gate.log" 2>&1 || status=$?
    if [[ "$expected" == error ]]; then
        if [[ "$status" -eq 0 ]] || grep -Fxq 'ci_red=false' "$scratch/output"; then
            problem='the gate must stop without reporting green CI'
        fi
    elif [[ "$status" -ne 0 ]]; then
        problem="the gate exited with status $status"
    elif ! grep -Fxq "ci_red=$expected" "$scratch/output"; then
        problem="expected ci_red=$expected"
    elif [[ "$expected" == true ]] && ! { grep -Fxq 'should_release=false' "$scratch/output" &&
        grep -Fxq 'reason=ci_red' "$scratch/output"; }; then
        problem='a red gate must refuse the release with reason=ci_red'
    fi
    if [[ -n "$problem" ]]; then
        failures=$((failures + 1))
        echo "FAIL: $name: $problem" >&2
        sed 's/^/    /' "$scratch/output" "$scratch/gate.log" >&2
    else
        echo "PASS: $name"
    fi
}

# Every slot is gated on its own newest completed run. Pull-request runs and
# manual runs of another commit never stand in for it.
for check in tests.yml:push clippy.yml:push tests.yml:schedule; do
    workflow="${check%:*}"
    event="${check#*:}"
    for conclusion in "${reds[@]}" success absent; do
        slots=()
        for slot in tests.yml:push clippy.yml:push tests.yml:schedule; do
            [[ "$slot" == "$check" ]] || slots+=("$(run "${slot%:*}" "${slot#*:}" success "$older")")
        done
        slots+=("$(run "$workflow" workflow_dispatch success "$newest" "$other_sha")"
            "$(run "$workflow" pull_request success "$newest")")
        if [[ "$conclusion" != absent ]]; then
            opposite=success
            [[ "$conclusion" != success ]] || opposite=failure
            slots+=("$(run "$workflow" "$event" "$conclusion" "$older")"
                "$(run "$workflow" "$event" "$opposite" "$oldest")")
        fi
        expected=true
        [[ "$conclusion" != success && "$conclusion" != absent ]] || expected=false
        scenario "$check / $conclusion" "$expected" "${slots[@]}"
    done
done

# A manual tests.yml run of the gated commit decides the scheduled slot when it
# is not older than the scheduled run.
red_schedule="$(run tests.yml schedule failure "$older")"
green_schedule="$(run tests.yml schedule success "$older")"
scenario 'manual green on HEAD newer than a red schedule releases' false \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$newer")"
scenario 'manual green on another commit leaves a red schedule red' true \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$newer" "$other_sha")"
scenario 'manual green on another branch leaves a red schedule red' true \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$newer" "$head_sha" feature)"
scenario 'manual green older than a red schedule leaves it red' true \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$oldest")"
scenario 'manual green tied with a red schedule leaves it red' true \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$older")"
for conclusion in "${reds[@]}"; do
    scenario "manual $conclusion on HEAD newer than a green schedule blocks" true \
        "${green_pushes[@]}" "$green_schedule" "$(run tests.yml workflow_dispatch "$conclusion" "$newer")"
done
scenario 'manual failure tied with a green schedule blocks' true \
    "${green_pushes[@]}" "$green_schedule" "$(run tests.yml workflow_dispatch failure "$older")"
scenario 'manual failure older than a green schedule is superseded' false \
    "${green_pushes[@]}" "$green_schedule" "$(run tests.yml workflow_dispatch failure "$oldest")"
scenario 'manual failure on HEAD without scheduled history blocks' true \
    "${green_pushes[@]}" "$(run tests.yml workflow_dispatch failure "$newer")"
scenario 'manual green on HEAD without scheduled history releases' false \
    "${green_pushes[@]}" "$(run tests.yml workflow_dispatch success "$newer")"
scenario 'the newest manual run on HEAD decides: failure after success blocks' true \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$newer")" \
    "$(run tests.yml workflow_dispatch failure "$newest")"
scenario 'the newest manual run on HEAD decides: success after failure releases' false \
    "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch failure "$newer")" \
    "$(run tests.yml workflow_dispatch success "$newest")"

# A green manual run counts only if it ran both scheduled checks.
scenario 'manual green with fixture coverage skipped leaves a red schedule red' true \
    "${green_pushes[@]}" "$red_schedule" \
    "$(run tests.yml workflow_dispatch success "$newer" "$head_sha" v1.0-dev "$coverage_skipped")"
scenario 'manual green without the pin guard leaves a red schedule red' true \
    "${green_pushes[@]}" "$red_schedule" \
    "$(run tests.yml workflow_dispatch success "$newer" "$head_sha" v1.0-dev "$pin_guard_absent")"
scenario 'manual green with fixture coverage skipped does not block a green schedule' false \
    "${green_pushes[@]}" "$green_schedule" \
    "$(run tests.yml workflow_dispatch success "$newer" "$head_sha" v1.0-dev "$coverage_skipped")"

# The push slots stay independent of any manual run.
for check in tests.yml:push clippy.yml:push; do
    other=clippy.yml
    [[ "$check" == tests.yml:push ]] || other=tests.yml
    scenario "manual green on HEAD does not replace a red $check" true \
        "$(run "${check%:*}" push failure "$older")" "$(run "$other" push success "$older")" \
        "$green_schedule" \
        "$(run tests.yml workflow_dispatch success "$newest")" \
        "$(run clippy.yml workflow_dispatch success "$newest")"
done

# A query that fails must stop the gate; it is never read as missing history.
for query in push schedule workflow_dispatch 'run view'; do
    GH_FAIL="$query" scenario "a failing '$query' query stops the gate" error \
        "${green_pushes[@]}" "$red_schedule" "$(run tests.yml workflow_dispatch success "$newer")"
done

# The gate names the jobs a manual run must have passed; they must exist.
for job in 'Fixture coverage' 'Platform pin guard'; do
    if grep -Fq "\"$job\"" "$scratch/gate.sh" && grep -Fxq "    name: $job" "$workflows/tests.yml"; then
        echo "PASS: the gate requires the '$job' job of tests.yml"
    else
        failures=$((failures + 1))
        echo "FAIL: the gate must require the '$job' job, named as in tests.yml" >&2
    fi
done

if [[ "$failures" -ne 0 ]]; then
    echo "$failures weekly gate scenario(s) failed." >&2
    exit 1
fi
