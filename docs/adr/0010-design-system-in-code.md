# ADR 0010: A design system in code, enforced by lint

- Status: Accepted
- Date: 2026-09-28 (completed 2026-09-29)
- Amends: ADR 0001 register, **UI** (how views are styled)
- Docs: `docs/design-system.md`, `docs/design-inventory.md`

## Context

Views had accumulated their own spacings, fonts, colours and rules. On
macOS 26 that showed as real defects: a full-width rule under the list
header ran into the floating glass sidebar (oagc-0cw), SwiftUI's `.help`
tool tips did not appear in column headers or Settings, and sheets differed
in padding, title type and keyboard handling.

## Decision

- Tokens and components live in `macos/OpenAGC/Design/`: `Space`,
  `Radius`, `TypeRole`, `Tone` (status colours and task categories
  included), surfaces (`glassCapsule`, `card`, `columnHeader`) and
  components (`Dialog`, `CancelButton`, `CategoryChip`, `TipCard`,
  `CapsuleTabs`, …). Views outside `Design/` use them, not literals.
- `docs/design-system.md` is the reference for every pattern: surfaces,
  components, dialogs, rows with a due day, status text, the reader,
  composer and agent, and behaviour (act then undo, keys, dates,
  accessibility, focus, motion). `docs/design-inventory.md` records the
  audit of every view against it.
- **Enforced**: `scripts/design-lint.sh --strict` (literal padding, spacing
  and radii, raw `Divider()`, status colour literals, stray Cancel buttons)
  and `scripts/help-lint.py --strict` (every control has a `.hoverHelp`,
  none ends with a full stop) run in `scripts/test-macos.sh`.
- Hover descriptions use `.hoverHelp` (an AppKit tool tip over the
  control) instead of `.help`, which macOS 26 does not show in column
  headers or Settings; toolbar items keep `.help` via `ToolbarToolTips`.

## Consequences

- A new view that uses a literal fails the app's checks; exceptions are
  marked in the line (`// menu`, `// inline`, `// no-help: …`).
- The reader's HTML cannot use the Swift tokens; its CSS is documented to
  follow the same scale (tracked as oagc-068.5).
- Self-snapshots cannot capture glass or some SwiftUI content, so the
  design doc's snapshots show layout and colour, not materials.

## Alternatives considered

- **A written guide without lint**: drifted within days before.
- **SwiftUI only, no AppKit tool tips**: tips would be missing where the
  system does not show them.
