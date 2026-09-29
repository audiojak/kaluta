# ADR 0009: Tests and scratch runs are isolated inside the app's bundle id

- Status: Accepted
- Date: 2026-09-28 (extended 2026-09-29)
- Amends: none (engineering practice); relates to the guardrail "never touch
  the maintainer's real account"

## Context

The Swift tests run inside the app as their test host, and snapshots and
live checks run the same app binary: both share the bundle id
`ai.actual.openagc` with the maintainer's own OpenAGC. So by default they
share its preferences, Keychain service, data directory and macOS's saved
window state. Leaks happened: a snapshot resized the user's saved window; a
test host's window frame was written into the real preferences on CI's
fresh machines; a scratch run that closed its window made the user's app
open with none.

## Decision

A test host or scratch run (`-OpenAGCDataDirectory …`) is isolated at every
place state can go:

- **Data**: a scratch directory (`openagc-apptests-<pid>`, or the one
  given), removed afterwards; the demo mailbox, never a real account.
- **Keychain**: its own service (`ai.actual.openagc.tests` / `.scratch`),
  emptied at launch and quit.
- **Preferences**: every store takes defaults from
  `CoreClient.appDefaults()`, one throwaway suite per process;
  `UserDefaults.standard` is only read, for launch arguments.
- **Window state**: from the App's `init`, before any window exists,
  `setFrameAutosaveName`, `saveFrame(usingName:)` and `NSSplitView`'s
  `autosaveName` are swapped out so nothing is named or saved, and scenes
  neither save nor restore state (`restorationBehavior(.disabled)`).
- **Agents**: fake agents only (`-OpenAGCFakeAgents YES`, automatic under
  tests); no real CLI or cloud routine runs.
- **Checks**: `scripts/test-macos.sh` and `scripts/snapshot.sh` fail when a
  run changes the real preferences file (with a diff of what changed), and
  the source is linted for stray `UserDefaults.standard` writes.

## Consequences

- The maintainer's app can run while tests and snapshots run.
- Method swizzling of AppKit is confined to isolated runs; the shipping app
  is unaffected. It depends on AppKit's selectors staying the same, which
  the tests check.
- New persistent state must be routed through `appDefaults()` or the
  scratch directory; the checks catch misses.

## Alternatives considered

- **A separate bundle id for tests and scratch builds**: a second app
  identity, with its own Keychain access and signing to keep in step, and
  it would not catch leaks from the real build.
- **Clearing autosave names after windows appear**: too late; SwiftUI
  names and saves the main window's frame in `windowDidLoad`.
