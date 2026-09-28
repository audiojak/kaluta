#!/usr/bin/env bash
# Design-system lint (docs/design-system.md): outside macos/OpenAGC/Design/,
# SwiftUI views take spacing and radii from the tokens and draw no raw
# Divider(). Menus may use Divider() (they are separators there, not
# rules); mark such lines with `// menu`.
# Usage: scripts/design-lint.sh [--strict]   (--strict exits 1 on findings)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/macos/OpenAGC"
cd "$SRC" || exit 2

findings=$(grep -rnE \
  -e '\.padding\((\.[a-zA-Z]+, *)?[0-9]+(\.[0-9]+)?\)' \
  -e '\.padding\([0-9]+(\.[0-9]+)?\)' \
  -e 'cornerRadius: *[0-9]' \
  -e 'spacing: *[1-9][0-9]*(\.[0-9]+)?[,)]' \
  -e '(^|[^A-Za-z])Divider\(\)' \
  --include='*.swift' . \
  | grep -v '^\./Design/' \
  | grep -v '// menu' \
  | grep -v '^\./App/Snapshot\.swift:' || true)

if [[ -z "$findings" ]]; then
  echo "design-lint: clean"
  exit 0
fi
count=$(printf '%s\n' "$findings" | wc -l | tr -d ' ')
printf '%s\n' "$findings" | sed 's/^/design-lint: /'
echo "design-lint: $count finding(s); use Space/Radius tokens, InsetRule or PaneDivider (docs/design-system.md)"
[[ "${1:-}" == "--strict" ]] && exit 1
exit 0
