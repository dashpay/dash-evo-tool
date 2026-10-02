#!/usr/bin/env bash
# Exercise the workflow's release gate against offline CI run histories.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/weekly-gate-tests.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin"
sed -n '/^          red=false$/,/^          echo "ci_red=false"/p' \
    "$script_dir/../../.github/workflows/weekly-build.yml" > "$scratch/gate.sh"
test -s "$scratch/gate.sh"
cat > "$scratch/bin/gh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1 $2" == 'run list' ]]
shift 2
workflow='' event='' branch='' status='' limit='' query=''
while (($#)); do
    case "$1" in
        --workflow) workflow="$2" ;;
        --event) event="$2" ;;
        --branch) branch="$2" ;;
        --status) status="$2" ;;
        --limit) limit="$2" ;;
        --json) [[ "$2" == conclusion ]] ;;
        --jq) query="$2" ;;
        *) exit 2 ;;
    esac
    shift 2
done
[[ "$branch" == v1.0-dev && "$status" == completed && "$limit" == 1 ]]
jq --arg workflow "$workflow" --arg event "$event" \
    '[.[] | select(.workflow == $workflow and ($event == "" or .event == $event))] | .[:1]' \
    "$RUN_HISTORY" | jq -r "$query"
MOCK
chmod +x "$scratch/bin/gh"

for failed_workflow in tests.yml clippy.yml; do
    for conclusion in failure cancelled timed_out startup_failure success absent; do
        jq -n --arg workflow "$failed_workflow" --arg conclusion "$conclusion" '
            [{workflow: $workflow, event: "schedule", conclusion: "success"},
             {workflow: $workflow, event: "workflow_dispatch", conclusion: "success"},
             {workflow: $workflow, event: "pull_request", conclusion: "success"}]
            + if $conclusion == "absent" then [] else
              [{workflow: $workflow, event: "push", conclusion: $conclusion}]
              end' > "$scratch/history.json"
        : > "$scratch/output"
        env PATH="$scratch/bin:$PATH" RUN_HISTORY="$scratch/history.json" \
            REF_NAME=v1.0-dev FORCE=true MANUAL_VERSION=1.0.0 \
            GITHUB_OUTPUT="$scratch/output" bash -euo pipefail "$scratch/gate.sh"
        expected=true
        [[ "$conclusion" != success && "$conclusion" != absent ]] || expected=false
        if ! grep -Fxq "ci_red=$expected" "$scratch/output"; then
            cat "$scratch/output"
            echo "FAIL: $failed_workflow / $conclusion should set ci_red=$expected" >&2
            exit 1
        fi
        if [[ "$expected" == true ]]; then
            grep -Fxq 'should_release=false' "$scratch/output"
            grep -Fxq 'reason=ci_red' "$scratch/output"
        fi
        echo "PASS: $failed_workflow / $conclusion"
    done
done
