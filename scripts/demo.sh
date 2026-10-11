#!/usr/bin/env bash
# Run the Debug app as the workflow demo (docs/plans/design-review.md): the
# demo mailbox dated from now, mail arriving every few minutes, and agents
# that use the mail tools; in a throwaway data directory, never the user's
# accounts or Keychain items. Quit the app to end it; the directory goes.
# Usage: scripts/demo.sh [seconds between arrivals] [extra app args...]
# KEEP=dir reuses (or keeps) a data directory instead.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/build/DerivedData/Build/Products/Debug/Kaluta.app/Contents/MacOS/Kaluta"
[[ -x "$APP" ]] || { echo "demo: build the app first (scripts/test-macos.sh build)" >&2; exit 1; }
ARRIVALS="${1:-180}"
shift || true
if [[ -n "${KEEP:-}" ]]; then
  DATA="$KEEP"
  mkdir -p "$DATA"
else
  DATA="$(mktemp -d -t kaluta-demo)"
  trap 'rm -rf "$DATA"' EXIT
fi
# The real preferences, and OpenAGC's from before the rename.
REAL_PREFS=("$HOME/Library/Preferences/org.kaluta.Kaluta.plist" "$HOME/Library/Preferences/ai.actual.openagc.plist")
prefs_stamp() { for p in "${REAL_PREFS[@]}"; do stat -f %m "$p" 2>/dev/null || echo none; done; }
prefs_before=$(prefs_stamp)
app_running=$(pgrep -f '(Kaluta|OpenAGC).app/Contents/MacOS/' >/dev/null && echo yes || echo no)
echo "demo: data in $DATA; mail arrives every ${ARRIVALS}s; quit Kaluta to end"
"$APP" -KalutaDataDirectory "$DATA" -KalutaDemo YES -KalutaFakeAgents YES -KalutaDemoWorkflows YES \
  -KalutaDemoArrivalSeconds "$ARRIVALS" "$@" >/dev/null 2>"${DEMO_LOG:-/dev/null}"
sleep 2 # cfprefsd writes just after the app quits
if [[ "$(prefs_stamp)" != "$prefs_before" && $app_running == no ]]; then
  echo "demo: the run wrote the real preferences (${REAL_PREFS[*]})" >&2
  exit 1
fi
