#!/usr/bin/env bash
# Remove scratch directories that earlier test runs left in $TMPDIR (and
# their throwaway preference files):
# `kaluta-*` (Rust tests) and UUID-named Kaluta data directories (Swift
# tests), when older than an hour so a concurrent run is never disturbed.
# Called by gate.sh and test-macos.sh; safe to run by hand.
set -euo pipefail
TMP="${TMPDIR:-/tmp}"
# (and `openagc-*`, as runs before the project was named Kaluta left them)
find "$TMP" -maxdepth 1 \( -name 'kaluta-*' -o -name 'openagc-*' \) -mmin +60 -exec rm -rf {} + 2>/dev/null || true
find "$TMP" -maxdepth 1 -type d -mmin +60 \
  -regex '.*/[0-9A-F]\{8\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{12\}' 2>/dev/null |
  while IFS= read -r dir; do
    if [[ -d "$dir/accounts" || -f "$dir/mail.sqlite" || -d "$dir/agents" ]]; then rm -rf "$dir"; fi
  done
# Throwaway preference suites from the app's tests (kaluta-tests-<UUID>,
# kaluta-scratch-<UUID>, org.kaluta.Kaluta.tests.<UUID>, and the same under
# the old name); never the app's own.
find "$HOME/Library/Preferences" -maxdepth 1 -mmin +60 \
  -regex '.*/\(kaluta-tests-\|kaluta-scratch-\|org\.kaluta\.Kaluta\.tests\.\|openagc-tests-\|openagc-scratch-\|ai\.actual\.openagc\.tests\.\)[0-9A-F]\{8\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{4\}-[0-9A-F]\{12\}\.plist' \
  -delete 2>/dev/null || true
