#!/bin/sh
# Phase 0 — stand up a local platform network on dashmate v3 (protocol 11).
#
# Why v3: v3 Drive (grovedb 4) only emits GroveDBProof::V0, which DET 0.9.3's
# SDK (platform 2.1, grovedb 3.1) can decode. That keeps the 0.9.3 baseline
# working with no time pressure. The network is later upgraded to v4 in
# 04_upgrade_to_v4.sh, which is the actual v3->v4 protocol-11->12 migration.
#
# Requires: docker running, npx available. Wipes any existing ~/.dashmate.
set -e

DASHMATE_V3="${DASHMATE_V3:-dashmate@3.0.2}"   # last stable v3 line (3.1 never released)
NODES="${NODES:-3}"

echo ">> Removing any existing local network + ~/.dashmate"
docker ps -a --format '{{.Names}}' | grep '^dashmate_' | xargs -r docker rm -f >/dev/null 2>&1 || true
docker volume ls --format '{{.Name}}' | grep '^dashmate_' | xargs -r docker volume rm >/dev/null 2>&1 || true
rm -rf ~/.dashmate

echo ">> dashmate setup local (${DASHMATE_V3}, ${NODES} masternodes)"
npx -y "$DASHMATE_V3" setup local --node-count="$NODES" --no-debug-logs --miner-interval=1m --verbose

# Fast, steady platform blocks so DKG/sync and manual UI steps don't stall.
for cfg in local_1 local_2 local_3 local_seed; do
  npx -y "$DASHMATE_V3" config set --config="$cfg" \
    platform.drive.tenderdash.consensus.createEmptyBlocksInterval 10s
done

echo ">> Starting group"
npx -y "$DASHMATE_V3" group start --verbose

echo ">> Waiting for platform blocks to flow"
until curl -s --max-time 3 http://127.0.0.1:46657/status \
      | python3 -c "import json,sys; sys.exit(0 if json.load(sys.stdin)['sync_info']['latest_block_height']!='1' else 1)" 2>/dev/null; do
  sleep 5
done
curl -s http://127.0.0.1:46657/status | python3 -c "import json,sys; d=json.load(sys.stdin); print('platform height', d['sync_info']['latest_block_height'], 'protocol', d['node_info']['protocol_version']['app'])"
echo ">> v3 network ready (expect protocol 11)."
