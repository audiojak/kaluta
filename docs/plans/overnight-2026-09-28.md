# Overnight run — 2026-09-28

Branch `overnight-4`, created from `main` after PR #3 was merged. Commit
and push after every closed issue. Open a PR to `main` in the morning;
never push to `main` overnight.

## Order

1. **Gmail categories as Inbox tabs** (spec §14.3 amendment 2026-09-28,
   categories). Narrowed listings `INBOX+CATEGORY_…` in the store (like
   `INBOX+IMPORTANT`), Primary = `CATEGORY_PERSONAL` or no category; tab
   bar above the list with unread counts, only categories that have mail;
   per-account remembered tab; "Show Categories" off switch; works with
   Important-only and search (search ignores tabs). Demo mailbox seeds
   categories so snapshots show them.
2. **Junk / Not Junk** (§14.3 amendment, junk). Core user actions that may
   set `SPAM` (and remove `INBOX`), undoable with exact diffs (§14.6a),
   outbox to Gmail; toolbar button, Message menu ⇧⌘J, context menu, the
   Spam mailbox offers Not Junk. `modify_labels` and agents still refuse
   `SPAM`.
3. **Undo for agent sends** (§14.6a amended). The approval card shows
   "Sending… Undo" for the hold; undo cancels the held send and reopens
   the draft in the review composer; no notice in the demo (sends at once).
4. **Finish test isolation.** Every `UserDefaults.standard` reached from
   tests goes through injected defaults (`ReaderStore` remote-image allow
   list, `LabelExpansion`, `GoogleClientFields`, `@AppStorage` settings,
   `NewMailNotifier` keys); test Keychain services are deleted after use
   (`KeychainTests` and the test host's `ai.actual.openagc.tests`); a test
   asserts the real prefs plist is untouched across the Swift suite.
5. **Large-mailbox test for tiered download.** A fake provider and fake
   IMAP with 100,000 messages over two years; measure time to a browsable
   list (headers pass), queue sizes per tier, peak memory, store size;
   record numbers in the spec (§7.4) and keep an `#[ignore]` benchmark
   plus a fast 5,000-message regression test in the gate.
6. If time remains: **list filters** (§14.3 amendment, filters): Unread,
   Starred, With Attachments from a header button.
7. If time remains: **New Message at the reader's leading edge**, as in
   Mail (an AppKit toolbar item if SwiftUI placement cannot do it).

## Checks before closing an issue

`scripts/gate.sh` (never piped) and `scripts/test-macos.sh test`. Record
spec amendments when a decision changes. List decisions in the handoff.
Run a review agent over `main..overnight-4` before the handoff and fix
what it confirms.

## Guardrails (in addition to CLAUDE.md)

- The maintainer's app may be running against the real account. Do not
  launch the app against the real account and never start sync outside the
  fakes. Snapshots only via `scripts/snapshot.sh` (scratch data directory,
  demo mailbox, throwaway preferences since 2026-09-27). Leave a running
  OpenAGC alone; `scripts/test-macos.sh` is safe (scratch dir, throwaway
  defaults).
- Nothing connects to Gmail, IMAP, Google identity endpoints or any real
  account; tests use `FakeProvider` and `provider_gmail::imap_fake`.
- Never read `~/.claude/.credentials.json`, `~/.codex/auth.json`, or the
  Keychain items under service `ai.actual.openagc`.
- Do not delete anything under `~/Library/Application Support/OpenAGC`.
- After the run, compare mtimes of the real `accounts/index.json`,
  `~/Library/Logs/OpenAGC/core.log` and
  `~/Library/Preferences/ai.actual.openagc.plist` against the start.
- Watch disk space (`df -h /System/Volumes/Data`); `target/debug` can be
  removed if space runs low.

## Morning handoff

PR to `main`, closed issues, spec amendments, decisions, anything
half-done with its `bd` notes, CI status, guardrail check results.
