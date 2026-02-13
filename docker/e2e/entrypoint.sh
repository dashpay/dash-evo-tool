#!/usr/bin/env bash
# Dash Evo Tool — E2E Test Entrypoint
#
# Orchestrates: Xvfb → tauri-driver → app launch → WebdriverIO tests → cleanup
# Exit code reflects test pass/fail.

set -euo pipefail

TAURI_DRIVER_PORT="${TAURI_DRIVER_PORT:-4444}"
DISPLAY="${DISPLAY:-:99}"
APP_BINARY="./src-tauri/target/debug/dash-evo-tool-tauri"
TEST_RESULTS_DIR="/app/test-results"
STARTUP_TIMEOUT=60

export DISPLAY

echo "=== Dash Evo Tool — Full E2E Test Runner ==="
echo "  Display:       $DISPLAY"
echo "  Driver port:   $TAURI_DRIVER_PORT"
echo "  App binary:    $APP_BINARY"
echo ""

# Ensure test results directory exists
mkdir -p "$TEST_RESULTS_DIR"

# ---- Cleanup handler ----
cleanup() {
    echo ""
    echo "=== Cleaning up ==="
    # Kill app if running
    if [ -n "${APP_PID:-}" ] && kill -0 "$APP_PID" 2>/dev/null; then
        echo "  Stopping app (PID $APP_PID)..."
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    # Kill frontend dev server if running
    if [ -n "${DEV_SERVER_PID:-}" ] && kill -0 "$DEV_SERVER_PID" 2>/dev/null; then
        echo "  Stopping frontend server (PID $DEV_SERVER_PID)..."
        kill "$DEV_SERVER_PID" 2>/dev/null || true
        wait "$DEV_SERVER_PID" 2>/dev/null || true
    fi
    # Kill tauri-driver if running
    if [ -n "${DRIVER_PID:-}" ] && kill -0 "$DRIVER_PID" 2>/dev/null; then
        echo "  Stopping tauri-driver (PID $DRIVER_PID)..."
        kill "$DRIVER_PID" 2>/dev/null || true
        wait "$DRIVER_PID" 2>/dev/null || true
    fi
    # Kill Xvfb if running
    if [ -n "${XVFB_PID:-}" ] && kill -0 "$XVFB_PID" 2>/dev/null; then
        echo "  Stopping Xvfb (PID $XVFB_PID)..."
        kill "$XVFB_PID" 2>/dev/null || true
        wait "$XVFB_PID" 2>/dev/null || true
    fi
    echo "  Done."
}
trap cleanup EXIT

# ---- 0. Start dbus (needed for Playwright Chromium in Phase 3) ----
eval $(dbus-launch --sh-syntax) 2>/dev/null || true

# ---- 1. Start Xvfb (virtual display) ----
echo "=== Starting Xvfb on $DISPLAY ==="
Xvfb "$DISPLAY" -screen 0 1280x800x24 -ac &
XVFB_PID=$!
sleep 1

# Verify Xvfb is running
if ! kill -0 "$XVFB_PID" 2>/dev/null; then
    echo "ERROR: Xvfb failed to start"
    exit 1
fi
echo "  Xvfb started (PID $XVFB_PID)"

# ---- 2. Start tauri-driver (WebDriver server) ----
echo "=== Starting tauri-driver on port $TAURI_DRIVER_PORT ==="
tauri-driver --port "$TAURI_DRIVER_PORT" &
DRIVER_PID=$!
sleep 2

# Verify tauri-driver is running
if ! kill -0 "$DRIVER_PID" 2>/dev/null; then
    echo "ERROR: tauri-driver failed to start"
    exit 1
fi
echo "  tauri-driver started (PID $DRIVER_PID)"

# ---- 3. Verify app binary exists ----
if [ ! -f "$APP_BINARY" ]; then
    echo "ERROR: App binary not found at $APP_BINARY"
    echo "  Did the Tauri build succeed?"
    exit 1
fi
echo "=== App binary found: $APP_BINARY ==="

# ---- 4. Verify frontend build exists ----
if [ ! -d "/app/dist" ] || [ ! -f "/app/dist/index.html" ]; then
    echo "ERROR: Frontend build not found in /app/dist/"
    echo "  Did 'npm run build' succeed in the Dockerfile?"
    exit 1
fi

# ---- 5. Start frontend dev server ----
# Debug builds use devUrl (http://localhost:1420) instead of embedding dist/.
# Serve the built frontend assets so the Tauri WebView can load them.
DEV_SERVER_PORT=1420
echo "=== Starting frontend static server on port $DEV_SERVER_PORT ==="
PORT=$DEV_SERVER_PORT node /app/docker/e2e/static-server.cjs &
DEV_SERVER_PID=$!
sleep 1

if ! kill -0 "$DEV_SERVER_PID" 2>/dev/null; then
    echo "ERROR: Frontend static server failed to start"
    exit 1
fi
echo "  Frontend server started (PID $DEV_SERVER_PID)"

# ---- 6. Clear SPV cache ----
echo "=== Clearing SPV cache (avoid stale segment data from prior builds) ==="
rm -rf /root/.config/Dash-Evo-Tool/spv/*
echo "  SPV cache cleared"

# ---- 7. Run WebdriverIO tests ----
echo "=== Running WebdriverIO E2E tests ==="
echo ""

# WebdriverIO will launch the app via tauri-driver using the binary path
# Tests are in tests/e2e-full/
TEST_EXIT_CODE=0
npx wdio tests/e2e-full/wdio.conf.ts \
    --baseUrl "http://localhost:$TAURI_DRIVER_PORT" \
    2>&1 | tee "$TEST_RESULTS_DIR/wdio-output.log" || TEST_EXIT_CODE=$?

echo ""
echo "=== Test run complete ==="
echo "  Exit code: $TEST_EXIT_CODE"

if [ "$TEST_EXIT_CODE" -eq 0 ]; then
    echo "  Result: ALL TESTS PASSED"
else
    echo "  Result: TESTS FAILED"
    echo "  See $TEST_RESULTS_DIR/ for details"
fi

exit "$TEST_EXIT_CODE"
