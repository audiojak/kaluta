#!/usr/bin/env bash
# Regenerate the Xcode project and run the app's tests.
# Output is filtered to errors, warnings from our sources and test results.
# Usage: scripts/test-macos.sh [build|test] [extra xcodebuild args...]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ACTION="${1:-test}"
shift || true

"$ROOT/scripts/clean-test-scratch.sh"
# Design-system rules that a grep can check (docs/design-system.md).
"$ROOT/scripts/design-lint.sh" --strict
# Every button, menu button, toggle and picker has a hover description.
"$ROOT/scripts/help-lint.py" --strict
# Test isolation: settings go through CoreClient.appDefaults() (a scratch
# suite under tests and snapshots); .standard is only for reading launch
# arguments, in the files listed here.
leaks=$(grep -rnE 'UserDefaults\.standard' --include='*.swift' "$ROOT/macos/Kaluta" \
  | grep -vE '/(Core/CoreClient|App/KalutaApp|App/Snapshot)\.swift:' || true)
appstorage=$(grep -rn '@AppStorage(' --include='*.swift' "$ROOT/macos/Kaluta" | grep -v 'store:' || true)
if [[ -n "$leaks$appstorage" ]]; then
  printf '%s\n%s\n' "$leaks" "$appstorage" | sed '/^$/d; s/^/isolation: /'
  echo "isolation: use CoreClient.appDefaults() instead of the real preferences"
  exit 1
fi
mkdir -p "$ROOT/build"
cd "$ROOT/macos"
# Optional per-developer signing overrides (scripts/dev-signing.sh).
[[ -f Local.xcconfig ]] || printf '// Optional overrides; see scripts/dev-signing.sh\n' > Local.xcconfig
xcodegen generate --spec project.yml --quiet

# The tests keep their scratch data under $TMPDIR/kaluta-apptests-<pid>
# (CoreClient.testScratchRoot); remove the ones this run made afterwards.
MARKER="$(mktemp "${TMPDIR:-/tmp}/kaluta-run-marker.XXXXXX")"
# Throwaway preference suites the tests made (UserDefaults(suiteName:)
# writes a plist) go too; only UUID-named ones, never the app's own.
# cfprefsd writes them just after the test host exits, hence the pause;
# the sweeper (clean-test-scratch.sh) catches any written later still.
UUID_RE='[0-9A-F]\{8\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{12\}'
cleanup() {
  find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'kaluta-apptests-*' -newer "$MARKER" -exec rm -rf {} + 2>/dev/null || true
  sleep 2
  find "$HOME/Library/Preferences" -maxdepth 1 -newer "$MARKER" \
    -regex ".*/\(kaluta-tests-\|kaluta-scratch-\|org\.kaluta\.Kaluta\.tests\.\|openagc-tests-\|openagc-scratch-\|ai\.actual\.openagc\.tests\.\)$UUID_RE\.plist" \
    -delete 2>/dev/null || true
  rm -f "$MARKER" "${prefs_copy:-}"
}
trap cleanup EXIT

# The real preferences must come through a test run untouched. The
# maintainer's own Kaluta may be running and write them itself; then the
# check can only warn.
# OpenAGC's, from before the project was named Kaluta, are watched too.
REAL_PREFS=("$HOME/Library/Preferences/org.kaluta.Kaluta.plist" "$HOME/Library/Preferences/ai.actual.openagc.plist")
prefs_stamp() { for p in "${REAL_PREFS[@]}"; do stat -f %m "$p" 2>/dev/null || echo none; done; }
prefs_dump() { for p in "${REAL_PREFS[@]}"; do echo "== $p"; plutil -p "$p" 2>/dev/null || true; done; }
prefs_before=$(prefs_stamp)
prefs_copy=$(mktemp -t kaluta-prefs)
prefs_dump >"$prefs_copy"
app_running=$(pgrep -f '(Kaluta|OpenAGC).app/Contents/MacOS/' >/dev/null && echo yes || echo no)

set +e
xcodebuild -project Kaluta.xcodeproj -scheme Kaluta \
  -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath "$ROOT/build/DerivedData" \
  "$ACTION" "$@" >"$ROOT/build/xcodebuild.log" 2>&1
status=$?
set -e

grep -E "error:|$ROOT/macos/.*warning:|\*\* (BUILD|TEST) |✔ Test |✘" "$ROOT/build/xcodebuild.log" \
  | grep -v -e 'appintentsmetadataprocessor' -e 'com.apple.linkd' || true
[[ $status -eq 0 ]] || echo "xcodebuild failed ($status); full log: build/xcodebuild.log"
if [[ "$ACTION" == test ]]; then
  sleep 2 # cfprefsd writes just after the host exits
  prefs_after=$(prefs_stamp)
  if [[ "$prefs_before" != "$prefs_after" ]]; then
    if [[ $app_running == yes ]]; then
      echo "isolation: warning: the real preferences changed, but Kaluta or OpenAGC was running and may have written them"
    else
      echo "isolation: the test run wrote the real preferences (${REAL_PREFS[*]})"
      # Name what was written, so the leak can be found.
      prefs_dump | diff "$prefs_copy" - | sed 's/^/isolation:   /' | head -40
      [[ $status -eq 0 ]] && status=1
    fi
  fi
fi
exit $status
