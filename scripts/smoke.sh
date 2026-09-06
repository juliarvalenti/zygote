#!/usr/bin/env bash
# Headless smoke test: build every project, render N frames of each, and fail
# if the renderer logged an error or the frame came out (nearly) black.
#
#   scripts/smoke.sh            # all projects
#   scripts/smoke.sh demo haze  # a subset
#
# Uses Xvfb + Mesa lavapipe when no display is present (Linux); on a desktop
# it renders on the real GPU. Needs ImageMagick's `identify` for the
# brightness check (skipped with a warning if absent). Works with macOS's
# bash 3.2 and without `timeout`.
set -uo pipefail
cd "$(dirname "$0")/.."

PROJECTS=("$@")
if [ ${#PROJECTS[@]} -eq 0 ]; then
  PROJECTS=(demo haze mycelium scope)
fi
OUT=${SMOKE_OUT:-target/smoke}
mkdir -p "$OUT"
FRAMES=${SMOKE_FRAMES:-90}

# Per-project brightness floor (fraction of full white; a black frame is ~0).
# Mycelium is dark by design and the scope is a thin trace on black, so
# their floors are lower.
floor_for() {
  case "$1" in
    demo) echo "${SMOKE_MIN_MEAN:-0.10}" ;;
    haze) echo "${SMOKE_MIN_MEAN:-0.05}" ;;
    mycelium) echo "${SMOKE_MIN_MEAN:-0.03}" ;;
    scope) echo "${SMOKE_MIN_MEAN:-0.004}" ;;
    *) echo "${SMOKE_MIN_MEAN:-0.02}" ;;
  esac
}

# `timeout` is GNU; macOS only has it as `gtimeout` from Homebrew coreutils.
TIMEOUT=$(command -v timeout || command -v gtimeout || true)

pkgs=()
for p in "${PROJECTS[@]}"; do pkgs+=(-p "zygote-$p"); done
echo "building: ${PROJECTS[*]/#/zygote-}"
cargo build ${SMOKE_RELEASE:+--release} "${pkgs[@]}" || exit 1
PROFILE=${SMOKE_RELEASE:+release}; PROFILE=${PROFILE:-debug}

runner=()
if [ -z "${DISPLAY:-}" ] && command -v xvfb-run >/dev/null; then
  runner=(xvfb-run -a -s "-screen 0 1280x720x24")
  export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
  if [ -f /usr/share/vulkan/icd.d/lvp_icd.json ]; then
    export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
  fi
fi

fail=0
for p in "${PROJECTS[@]}"; do
  bin="target/$PROFILE/zygote-$p"
  png="$OUT/$p.png"
  log="$OUT/$p.log"
  rm -f "$png"
  echo "--- $p"
  RUST_LOG=${RUST_LOG:-error,zygote_render=warn} \
    ${TIMEOUT:+"$TIMEOUT" 300} ${runner[@]+"${runner[@]}"} "$bin" \
    --port $((9600 + RANDOM % 200)) --capture "$png" --frames "$FRAMES" > "$log" 2>&1
  if grep -E "ERROR|panicked" "$log" | grep -v XSETTINGS; then
    echo "FAIL $p: errors in log"; fail=1; continue
  fi
  if [ ! -f "$png" ]; then
    echo "FAIL $p: no capture written"; fail=1; continue
  fi
  if command -v identify >/dev/null; then
    mean=$(identify -format "%[fx:mean]" "$png")
    floor=$(floor_for "$p")
    if awk "BEGIN{exit !($mean < $floor)}"; then
      echo "FAIL $p: frame too dark (mean $mean < floor $floor)"; fail=1; continue
    fi
    echo "ok   $p: mean brightness $mean (floor $floor) → $png"
  else
    echo "ok   $p: captured $png (install ImageMagick for the brightness check)"
  fi
done
exit $fail
