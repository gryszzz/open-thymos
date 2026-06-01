#!/usr/bin/env bash
#
# OpenThymos quickstart: build the runtime, start it, and drive one real run
# end-to-end — Intent -> Proposal -> Commit — then shut it down.
#
#   ./scripts/quickstart.sh
#   ./scripts/quickstart.sh "Map the repo and summarize the runtime"
#
# Connect an AI by exporting a key first (otherwise it runs the mock provider):
#   ANTHROPIC_API_KEY=sk-ant-... ./scripts/quickstart.sh
#   OPENAI_API_KEY=sk-...        ./scripts/quickstart.sh
# Force a provider/model: THYMOS_DEFAULT_PROVIDER=openai THYMOS_DEFAULT_MODEL=gpt-4o-mini
set -euo pipefail

TASK="${1:-Map the repo and summarize the execution runtime}"
PORT="${THYMOS_PORT:-3001}"
BASE="http://localhost:${PORT}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/thymos"

echo "==> Building thymos-server (first build can take a minute)…"
cargo build -p thymos-server

echo "==> Starting server on :${PORT} …"
# Ephemeral state in a temp dir so the quickstart leaves nothing behind.
TMP="$(mktemp -d)"
THYMOS_BIND_ADDR="0.0.0.0:${PORT}" \
THYMOS_DB_PATH="$TMP/runs.db" \
THYMOS_GATEWAY_DB_PATH="$TMP/gw.db" \
THYMOS_MARKETPLACE_DB_PATH="$TMP/market.db" \
  ./target/debug/thymos-server >"$TMP/server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

echo -n "==> Waiting for /health "
for _ in $(seq 1 30); do
  if curl -sf "${BASE}/health" >/dev/null 2>&1; then break; fi
  echo -n "."; sleep 1
done
echo

HEALTH="$(curl -s "${BASE}/health")"
PROVIDER="$(printf '%s' "$HEALTH" | sed -n 's/.*"default_provider":"\([^"]*\)".*/\1/p')"
echo "==> Active provider: ${PROVIDER:-unknown}"
if [ "${PROVIDER:-mock}" = "mock" ]; then
  echo "    (mock — export ANTHROPIC_API_KEY or OPENAI_API_KEY for a real model)"
fi

echo "==> Submitting task: \"${TASK}\""
RESP="$(curl -s -X POST "${BASE}/runs" \
  -H 'content-type: application/json' \
  -d "$(printf '{"task": %s, "max_steps": 8}' "$(printf '%s' "$TASK" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')")")"
RUN_ID="$(printf '%s' "$RESP" | sed -n 's/.*"run_id":"\([^"]*\)".*/\1/p')"
if [ -z "$RUN_ID" ]; then
  echo "!! Could not start run. Response: $RESP"; exit 1
fi
echo "==> Run started: ${RUN_ID}"

echo "==> Running (Intent -> Proposal -> Commit). Live tokens: ${BASE}/runs/${RUN_ID}/execution/stream"
STATUS="running"
for _ in $(seq 1 60); do
  SNAP="$(curl -s "${BASE}/runs/${RUN_ID}" || true)"
  STATUS="$(printf '%s' "$SNAP" | sed -n 's/.*"status":"\([a-z_]*\)".*/\1/p' | head -1)"
  case "$STATUS" in completed|failed) break ;; esac
  echo -n "."; sleep 1
done
echo
echo "==> Run status: ${STATUS:-unknown}"

echo "==> Final world projection:"
curl -s "${BASE}/runs/${RUN_ID}/world" || true
echo
echo "==> Done. (ephemeral state removed on exit)"
