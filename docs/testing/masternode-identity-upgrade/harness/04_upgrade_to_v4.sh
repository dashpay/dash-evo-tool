#!/bin/sh
# Phase 2 — upgrade the SAME running network from v3 to v4 in place.
#
# dashmate v4 migrates the config file (configFormatVersion 3 -> 4) and bumps
# every image tag to v4 (drive:4, tenderdash 1.6.0, rs-dapi:4). v4 Drive reads
# the v3 on-disk state (the mainnet-supported upgrade path). v4 validators
# signal protocol 12, which activates at the next epoch boundary; from then on
# Drive emits GroveDBProof::V1 and DET 0.9.3 can no longer decode proofs
# (expected — mirrors a real mainnet upgrade).
#
# BACK UP FIRST if you want to be able to return to the v3 state:
#   tar czf dashmate-home.tgz -C ~ .dashmate
#   for v in $(docker volume ls --format '{{.Name}}' | grep '^dashmate_'); do
#     docker run --rm -v "$v":/vol -v "$PWD":/b alpine tar czf "/b/$v.tgz" -C /vol .
#   done
set -e
DASHMATE_V4="${DASHMATE_V4:-dashmate@4.0.0}"

echo ">> Stopping v3 group"
npx -y dashmate@3.0.2 group stop --force || true

echo ">> Starting group with ${DASHMATE_V4} (migrates config + images)"
npx -y "$DASHMATE_V4" group start --verbose

echo ">> Waiting for protocol 12 to activate (next epoch boundary)"
attempt=1
max_attempts=60
until curl -s --max-time 3 http://127.0.0.1:46657/block \
      | python3 -c "import json,sys; sys.exit(0 if json.load(sys.stdin)['block']['header']['version']['app']=='12' else 1)" 2>/dev/null; do
  if [ "$attempt" -ge "$max_attempts" ]; then
    echo "ERROR: protocol 12 did not activate after ${max_attempts} attempts (about 30 minutes)." >&2
    exit 1
  fi
  curl -s --max-time 3 http://127.0.0.1:46657/block | python3 -c "import json,sys; h=json.load(sys.stdin)['block']['header']; print('height', h['height'], 'active', h['version']['app'], 'proposed', h.get('proposed_protocol_version'))" 2>/dev/null || true
  sleep 30
  attempt=$((attempt + 1))
done
echo ">> Network is now on protocol 12 (v4). Proofs are V1."
