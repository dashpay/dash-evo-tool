#!/usr/bin/env bash
# Render spike: launch the released GUI on Xvfb with software rendering,
# record start-up timing, idle CPU, screenshots and the chosen GPU adapter.
set -euo pipefail
APP="$1"; OUT="$2"; DISP="${3:-:99}"
IMG="$(dirname -- "$0")/img.py"
shot() { scrot -o "$1"; }
umask 077
mkdir -p "$OUT/shots"
# The secret store refuses a data dir under a group- or world-writable parent.
chmod 700 "$OUT"
export DISPLAY="$DISP"
export DASH_EVO_DATA_DIR="$OUT/data"
export RUST_LOG="${RUST_LOG:-info,wgpu_core=info,wgpu_hal=warn}"
mkdir -p "$DASH_EVO_DATA_DIR"
Xvfb "$DISP" -screen 0 1920x1080x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB=$!
trap 'kill "${APP_PID:-}" "$XVFB" 2>/dev/null || true' EXIT
for _ in $(seq 50); do xdpyinfo -display "$DISP" >/dev/null 2>&1 && break; sleep 0.1; done
t0=$(date +%s.%N)
"$APP" >"$OUT/app.stdout.log" 2>"$OUT/app.stderr.log" &
APP_PID=$!
wid=""
for _ in $(seq 1200); do
  wid=$(xdotool search --onlyvisible --pid "$APP_PID" 2>/dev/null | head -1 || true)
  [ -n "$wid" ] && break
  kill -0 "$APP_PID" 2>/dev/null || { echo "app exited before a window appeared" >&2; exit 1; }
  sleep 0.1
done
[ -n "$wid" ] || { echo "no window within 120 s" >&2; exit 1; }
t_window=$(date +%s.%N)
# First non-blank content: the screenshot's pixel spread rises above a flat fill.
t_content=""
for i in $(seq 600); do
  shot "$OUT/shots/probe.png"
  sd=$(python3 -I "$IMG" spread "$OUT/shots/probe.png")
  if awk -v s="$sd" 'BEGIN{exit !(s>5)}'; then t_content=$(date +%s.%N); break; fi
  sleep 0.2
done
mv "$OUT/shots/probe.png" "$OUT/shots/00-first-content.png"
sleep 5
shot "$OUT/shots/01-settled.png"
ticks() { awk '{print $14+$15}' "/proc/$APP_PID/stat"; }
c0=$(ticks); s0=$(date +%s.%N); sleep 20; c1=$(ticks); s1=$(date +%s.%N)
hz=$(getconf CLK_TCK)
idle_cpu=$(awk -v a="$c0" -v b="$c1" -v s="$s0" -v e="$s1" -v hz="$hz" 'BEGIN{printf "%.1f", 100*(b-a)/hz/(e-s)}')
# Interaction latency: a click, then time until the screen changes.
lat_report=""
n=1
for xy in "40 200" "40 300" "40 400" "40 500" "960 540"; do
  set -- $xy
  shot "$OUT/shots/before.png"
  ta=$(date +%s.%N)
  xdotool mousemove --sync "$1" "$2" click 1
  tb=""
  for _ in $(seq 100); do
    shot "$OUT/shots/after.png"
    if python3 -I "$IMG" differ "$OUT/shots/before.png" "$OUT/shots/after.png"; then tb=$(date +%s.%N); break; fi
  done
  lat=$( [ -n "$tb" ] && awk -v a="$ta" -v b="$tb" 'BEGIN{printf "%.2f", b-a}' || echo "no-change" )
  lat_report="$lat_report click($1,$2)=${lat}s"
  sleep 1
  shot "$OUT/shots/1$n-click.png"
  n=$((n+1))
done
rm -f "$OUT/shots/before.png" "$OUT/shots/after.png"
rss_mb=$(awk '/VmRSS/{printf "%.0f", $2/1024}' "/proc/$APP_PID/status")
adapter=$(grep -h -i -m3 -E 'adapter|llvmpipe|lavapipe|vulkan|backend' "$OUT/app.stderr.log" "$OUT/app.stdout.log" "$DASH_EVO_DATA_DIR"/*.log 2>/dev/null | head -3 | tr '\n' ' ' || true)
{
  echo "window_s=$(awk -v a="$t0" -v b="$t_window" 'BEGIN{printf "%.2f", b-a}')"
  echo "first_content_s=$( [ -n "$t_content" ] && awk -v a="$t0" -v b="$t_content" 'BEGIN{printf "%.2f", b-a}' || echo none)"
  echo "idle_cpu_percent=$idle_cpu"
  echo "rss_mb=$rss_mb"
  echo "click_latency=$lat_report"
  echo "adapter_log=$adapter"
} | tee "$OUT/timings.txt"
