# Overnight run — 2026-09-27

Branch `overnight-3`, created from `overnight-2` (PR #2 is not merged yet;
if it is merged before the run starts, branch from `main` instead). Commit
and push after every closed issue. Open a PR to `main` in the morning; never
push to `main` overnight.

## Order

1. **oagc-u40** Snippet shows a replacement character. Small; find the
   decoding path first (it may be the IMAP or REST snippet).
2. **oagc-m90** Tiered download (spec §7.4 amendment 2026-09-27): body
   window and headers-only queue priority, `ensure_bodies` for agents and
   the reader, search over header-only mail, snippets from a partial
   fetch, Settings.
3. **oagc-82v** Action acknowledgement and Undo (spec §14.6a): core undo
   tokens with exact label diffs, the window's UndoManager (Edit › Undo),
   the notice, every action path, Undo Send. Decision already taken:
   quitting with a send still in its undo window sends it immediately.
4. **oagc-gra** Agent suggestions (spec §14.6b): suggestion model, empty
   state, chips with keyboard navigation, recent prompts.
5. If time remains: **oagc-pe0** (tests remove their scratch dirs).

## Checks before closing an issue

`scripts/gate.sh` (never piped) and `scripts/test-macos.sh test`. Record
spec amendments when a decision changes. List decisions in the handoff.

## Guardrails (in addition to CLAUDE.md)

- The maintainer's app may be running against the real account. Do not
  launch the app against the real account and never start sync outside the
  fakes. Snapshots only via `scripts/snapshot.sh` (scratch data directory,
  demo mailbox). If OpenAGC is running when a build is needed, leave it
  alone and build anyway only if the user said it is quit; otherwise
  `scripts/test-macos.sh` is still safe (the test host uses a scratch dir).
- Nothing connects to Gmail, IMAP, Google identity endpoints or any real
  account; tests use `FakeProvider` and `provider_gmail::imap_fake`.
- Never read `~/.claude/.credentials.json`, `~/.codex/auth.json`, or the
  Keychain items under service `ai.actual.openagc`.
- Do not delete anything under `~/Library/Application Support/OpenAGC`.
- Test isolation: construct `AppModel` with a throwaway `UserDefaults`
  suite; never `UserDefaults.standard` or `KeychainSecretStore()` in tests.
- After the run, compare mtimes of the real `accounts/index.json`,
  `core.log` and the prefs plist against the start of the run.
- Watch disk space (`df -h /System/Volumes/Data`);
  `scripts/clean-test-scratch.sh` sweeps stale test directories.

## Morning handoff

PR to `main`, closed issues, spec amendments, decisions, anything
half-done with its `bd` notes, CI status.
