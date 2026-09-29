#!/usr/bin/env bash
# Capture a snapshot of the Debug app against the demo mailbox in a
# throwaway data directory, never the user's accounts or Keychain items.
# Usage: scripts/snapshot.sh out.png [extra -OpenAGCSnapshot* args...]
# SNAPSHOT_LOG=file keeps the app's stderr (the -OpenAGCSnapshotDumpViews tree).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
shift
APP="$ROOT/build/DerivedData/Build/Products/Debug/OpenAGC.app/Contents/MacOS/OpenAGC"
DATA="$(mktemp -d -t openagc-snapshot)"
trap 'rm -rf "$DATA"' EXIT
REAL_PREFS="$HOME/Library/Preferences/ai.actual.openagc.plist"
prefs_before=$(stat -f %m "$REAL_PREFS" 2>/dev/null || echo none)
# The user's own OpenAGC may write them itself.
app_running=$(pgrep -f 'OpenAGC.app/Contents/MacOS/OpenAGC' >/dev/null && echo yes || echo no)
"$APP" -OpenAGCDataDirectory "$DATA" -OpenAGCDemo YES -OpenAGCFakeAgents YES \
  -OpenAGCSnapshot "$OUT" "$@" >/dev/null 2>"${SNAPSHOT_LOG:-/dev/null}"
echo "snapshot: $OUT"
sleep 2 # cfprefsd writes just after the app quits
if [[ "$(stat -f %m "$REAL_PREFS" 2>/dev/null || echo none)" != "$prefs_before" && $app_running == no ]]; then
  echo "snapshot: the run wrote the real preferences ($REAL_PREFS)" >&2
  exit 1
fi
