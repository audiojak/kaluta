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
# Test isolation: settings go through CoreClient.appDefaults() (a scratch
# suite under tests and snapshots); .standard is only for reading launch
# arguments, in the files listed here.
leaks=$(grep -rnE 'UserDefaults\.standard' --include='*.swift' "$ROOT/macos/OpenAGC" \
  | grep -vE '/(Core/CoreClient|App/OpenAGCApp|App/Snapshot)\.swift:' || true)
appstorage=$(grep -rn '@AppStorage(' --include='*.swift' "$ROOT/macos/OpenAGC" | grep -v 'store:' || true)
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

# The tests keep their scratch data under $TMPDIR/openagc-apptests-<pid>
# (CoreClient.testScratchRoot); remove the ones this run made afterwards.
MARKER="$(mktemp "${TMPDIR:-/tmp}/openagc-run-marker.XXXXXX")"
# Throwaway preference suites the tests made (UserDefaults(suiteName:)
# writes a plist) go too; only UUID-named ones, never the app's own.
# cfprefsd writes them just after the test host exits, hence the pause;
# the sweeper (clean-test-scratch.sh) catches any written later still.
UUID_RE='[0-9A-F]\{8\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{12\}'
cleanup() {
  find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'openagc-apptests-*' -newer "$MARKER" -exec rm -rf {} + 2>/dev/null || true
  sleep 2
  find "$HOME/Library/Preferences" -maxdepth 1 -newer "$MARKER" \
    -regex ".*/\(openagc-tests-\|openagc-scratch-\|ai\.actual\.openagc\.tests\.\)$UUID_RE\.plist" \
    -delete 2>/dev/null || true
  rm -f "$MARKER"
}
trap cleanup EXIT

# The real preferences must come through a test run untouched. The
# maintainer's own OpenAGC may be running and write them itself; then the
# check can only warn.
REAL_PREFS="$HOME/Library/Preferences/ai.actual.openagc.plist"
prefs_before=$(stat -f %m "$REAL_PREFS" 2>/dev/null || echo none)
app_running=$(pgrep -f 'OpenAGC.app/Contents/MacOS/OpenAGC' >/dev/null && echo yes || echo no)

set +e
xcodebuild -project OpenAGC.xcodeproj -scheme OpenAGC \
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
  prefs_after=$(stat -f %m "$REAL_PREFS" 2>/dev/null || echo none)
  if [[ "$prefs_before" != "$prefs_after" ]]; then
    if [[ $app_running == yes ]]; then
      echo "isolation: warning: the real preferences changed, but OpenAGC was running and may have written them"
    else
      echo "isolation: the test run wrote the real preferences ($REAL_PREFS)"
      [[ $status -eq 0 ]] && status=1
    fi
  fi
fi
exit $status
