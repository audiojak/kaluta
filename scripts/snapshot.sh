#!/usr/bin/env bash
# Capture a snapshot of the Debug app against the demo mailbox in a
# throwaway data directory, never the user's accounts or Keychain items.
# Usage: scripts/snapshot.sh out.png [extra -KalutaSnapshot* args...]
# SNAPSHOT_LOG=file keeps the app's stderr (the -KalutaSnapshotDumpViews tree).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
shift
APP="$ROOT/build/DerivedData/Build/Products/Debug/Kaluta.app/Contents/MacOS/Kaluta"
DATA="$(mktemp -d -t kaluta-snapshot)"
trap 'rm -rf "$DATA"' EXIT
# The real preferences, and OpenAGC's from before the rename.
REAL_PREFS=("$HOME/Library/Preferences/org.kaluta.Kaluta.plist" "$HOME/Library/Preferences/ai.actual.openagc.plist")
prefs_stamp() { for p in "${REAL_PREFS[@]}"; do stat -f %m "$p" 2>/dev/null || echo none; done; }
prefs_before=$(prefs_stamp)
# The user's own Kaluta (or OpenAGC) may write them itself.
app_running=$(pgrep -f '(Kaluta|OpenAGC).app/Contents/MacOS/' >/dev/null && echo yes || echo no)
"$APP" -KalutaDataDirectory "$DATA" -KalutaDemo YES -KalutaFakeAgents YES \
  -KalutaSnapshot "$OUT" "$@" >/dev/null 2>"${SNAPSHOT_LOG:-/dev/null}"
echo "snapshot: $OUT"
sleep 2 # cfprefsd writes just after the app quits
if [[ "$(prefs_stamp)" != "$prefs_before" && $app_running == no ]]; then
  echo "snapshot: the run wrote the real preferences (${REAL_PREFS[*]})" >&2
  exit 1
fi
