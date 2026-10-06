#!/usr/bin/env bash
# Design-system lint (docs/design-system.md): outside macos/OpenAGC/Design/,
# SwiftUI views take spacing and radii from the tokens and draw no raw
# Divider(). Menus may use Divider() (they are separators there, not
# rules); mark such lines with `// menu`. Status colours come from `Tone`
# (failure, caution, approved), and dialogs use `CancelButton`, which
# answers Escape; a Cancel that is not a dialog's is marked `// inline`.
# Type comes from TypeRole (no raw .font(.callout) or systemFont(ofSize:)),
# dates from Design/DateStyle.swift, and tap-only rows carry a button trait.
# Usage: scripts/design-lint.sh [--strict]   (--strict exits 1 on findings)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/macos/OpenAGC"
cd "$SRC" || exit 2

findings=$(grep -rnE \
  -e '\.padding\((\.[a-zA-Z]+, *)?[0-9]+(\.[0-9]+)?\)' \
  -e '\.padding\([0-9]+(\.[0-9]+)?\)' \
  -e 'cornerRadius: *[0-9]' \
  -e '[sS]pacing: *[1-9][0-9]*(\.[0-9]+)?[,)]' \
  -e '\.foregroundStyle\(\.(orange|red|green)\)' \
  -e '(^|[^A-Za-z.])Color\.(orange|red|green)([^A-Za-z]|$)' \
  -e 'Button\("Cancel"' \
  -e '\.font\(\.(callout|caption2?|headline|body|subheadline|footnote|title[23]?|largeTitle)' \
  -e 'systemFont\(ofSize:' \
  -e '(DateFormatter|RelativeDateTimeFormatter)\(\)' \
  -e '(^|[^A-Za-z])Divider\(\)' \
  --include='*.swift' . \
  | grep -v '^\./Design/' \
  | grep -v '// menu' \
  | grep -v '// inline' \
  | grep -v '^\./App/Snapshot\.swift:' || true)

# A row that acts on a tap says so to VoiceOver: .onTapGesture with
# .accessibilityAddTraits(.isButton) within the next three lines.
taps=$(grep -rn --include='*.swift' -A3 '\.onTapGesture' . | grep -v '^\./Design/' \
  | awk -F'[:-]' '/\.onTapGesture/ { if (pending) print pending; pending=$0; n=0; next }
                  /isButton/ { pending=""; next }
                  { if (pending && ++n >= 3) { print pending; pending="" } }
                  END { if (pending) print pending }' \
  | grep -v 'count: 2' | grep -v '// tap-only' | sed 's/$/ (tap-only: add .accessibilityAddTraits(.isButton))/' || true)
[[ -n "$taps" ]] && findings=$(printf '%s\n%s' "$findings" "$taps" | sed '/^$/d')

# The reader's and composer's CSS keep to the spacing scale too: literal
# px in padding, margin, gap and border-radius must be one of the Space or
# Radius values (or come from them by interpolation).
css=$(grep -nE '(padding|margin|gap|border-radius)[^;{}]*[0-9]px' \
        Features/MessageView/EmailDocument.swift Features/Composer/ComposerView.swift \
  | while IFS= read -r line; do
      for v in $(printf '%s\n' "$line" | grep -oE '(padding|margin|gap|border-radius)(-[a-z]+)?:[^;}]*' \
                 | grep -oE '[0-9]+px' | tr -d 'px'); do
        if [[ " 0 2 4 6 8 12 16 20 24 32 " != *" $v "* ]]; then
          printf '%s (%spx off the scale)\n' "$line" "$v"
        fi
      done
    done)
[[ -n "$css" ]] && findings=$(printf '%s\n%s' "$findings" "$css" | sed '/^$/d')

if [[ -z "$findings" ]]; then
  echo "design-lint: clean"
  exit 0
fi
count=$(printf '%s\n' "$findings" | wc -l | tr -d ' ')
printf '%s\n' "$findings" | sed 's/^/design-lint: /'
echo "design-lint: $count finding(s); use Space/Radius/Tone tokens, InsetRule, PaneDivider or CancelButton (docs/design-system.md)"
[[ "${1:-}" == "--strict" ]] && exit 1
exit 0
