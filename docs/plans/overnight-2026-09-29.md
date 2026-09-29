# Overnight run — 2026-09-29

Branch `overnight-5`, created from `main` after PR #4 was merged. Commit
and push after every closed issue. Open a PR to `main` in the morning;
never push to `main` overnight.

Two goals: every design pattern in the app written down in the design
system, then a task-based way of working through email with AI, which
changes the look of the app as little as possible.

## Order

0. **CI green first** (oagc-aoz). CI's macOS job fails the isolation
   check: on a fresh runner the test host creates the real preferences
   file. `scripts/test-macos.sh` now prints the keys written; read them
   from the CI log, route the write through `CoreClient.appDefaults()`
   (or clear the autosave name before AppKit saves), push, and confirm
   the job is green.

### Step one: the design system covers everything (epic oagc-068)

1. **Pattern inventory** (oagc-068.1). Walk every view in `macos/OpenAGC`
   and list each visual or interaction pattern, and whether
   `docs/design-system.md` describes it. Known gaps to check at least:
   - surfaces: the reader's message cards (HTML/CSS: card, avatar
     initials and colours, collapsed and expanded states, "paper" bodies),
     the composer's header rows, quote pane and writing-help bar, the
     agent prompt strip under the reader, the agent column's transcript
     and approval cards, sheets and dialogs, the Settings panes, the Sync
     Debugger's grids;
   - components: `CapsuleTabs`, `TipCard`, `hoverHelp`/`ToolTipArea`,
     the undo notice, attachment tiles and strips, avatars
     (`AccountAvatar`), empty states, progress and error text;
   - behaviour: toolbar groups and `ToolbarHelp`, keyboard conventions
     (single keys in lists, ⌘ keys in menus, the shortcuts window),
     confirmation and undo (act, then offer Undo; confirm only what cannot
     be undone), date formats, accessibility labels, focus.
2. **Write them down** (oagc-068.2). Each pattern gets its entry: what it is,
   when to use it, the API, and the rules. Extend `design-lint.sh` for
   anything a grep can check. Refresh the snapshots (light and dark) and
   add ones for the composer and the agent column. Patterns the tasks
   work needs are defined here first: a **dialog** (a sheet with a
   default action on Return and Cancel on Escape), a **list row with a
   due date** (overdue in the caution tone; red stays for failure), and
   **category chips**.

### Step two: tasks from email (epic oagc-0pq)

Decisions (maintainer, 2026-09-29):

- **Categories:** a fixed starting set, editable in Settings: Reply,
  Decide, Gather Info, Schedule, Review, Admin, Follow Up. Claude
  picks from the account's list.
- **Where:** a **Tasks** entry in the sidebar under Favorites. Choosing
  it swaps the list column to tasks; the reader shows the task's email.
- **Accepting a task leaves the email where it is.** Nothing changes in
  Gmail; the task links to the thread and message.
- **Sending the reply a task called for marks it done**, with the undo
  notice to bring it back.

Design:

- **Store** (mail-store migration `0009_tasks`, per account): `tasks`
  (id, thread and message ids, title, notes, category, due date or none,
  suggested action (reply, reply all, forward, none), status (open, done),
  created, completed, source: AI or you) and `task_categories` (name,
  position). Local only: no Google Tasks (it would need another Google
  scope and account actions).
- **Core API** (openagc-core, FFI): create, update, complete, reopen,
  delete and list tasks (open first, by due date); categories get, set
  and reorder; `task_prompt(thread_ids)` builds the request from stored
  mail (each thread's latest message as plain text, capped) with today's
  date and the category list; `parse_task_suggestions(text)` reads
  Claude's JSON answer (title, category, due date or null, action, one
  line of why) and rejects anything malformed. Tested in Rust.
- **Asking Claude:** the app runs a one-turn agent session, scoped to the
  threads concerned (as the composer's writing help does), and refuses
  any proposal that would change mail. The fake agent learns to answer
  task prompts with fixed JSON, so every path is tested without a real
  agent.
- **One email, `t`:** in any mail list, `t` (and Message › New Task
  from Email) asks Claude about the selected thread and opens the
  **task dialog**: title, category, due date (Claude's guess, with a
  date picker), action, and the why line; Return accepts, Escape
  cancels, and a button asks again. While Claude works, the dialog shows
  progress; if no agent is ready, it opens empty for you to fill in.
- **Many emails, `⇧T`:** Message › Create Tasks… opens the **bulk
  sheet**: the highlighted threads, or else the latest 20 in the open
  list. One request for all; each row shows the email, Claude's title,
  category and due date, editable, with a checkbox; threads that already
  have an open task are marked and unchecked. "Add N Tasks".
- **Task list:** rows grouped Overdue, Today, This Week, Later, No Date;
  each shows the title, category chip, due date and the email's sender
  and subject, in the calm row style. Keys: `↩` edit, `r` reply, `a`
  reply all, `f` forward, `e` done (as Archive is in mail), `⌫` delete,
  `c` change category; the same as toolbar buttons and a context menu.
  Replying opens the composer with the task attached; sending completes
  the task (undoable). A done filter shows finished tasks.
- **Look:** no new window and no new chrome: the sidebar entry, the same
  list column, reader and toolbar, and the dialog/sheet patterns from
  step one.
- **Spec:** a new §14.8 Tasks, and the store (§8) gains the two tables.

Issues, in order:

3. **Tasks in the store and core** (oagc-0pq.1): migration, core API,
   categories with defaults, FFI, Rust tests.
4. **Claude's task suggestions** (oagc-0pq.2): prompt and parser in the core,
   the fake agent's JSON answers, the one-turn session in Swift, tests.
5. **`t`: the task dialog** (oagc-0pq.3): menu item, key, dialog, accept,
   ask again, no-agent fallback; tests and snapshots.
6. **The task list** (oagc-0pq.4): sidebar entry, list, groups, row, reader,
   keys, toolbar and context menu, done filter; reply completes the task
   with undo; demo tasks for snapshots.
7. **`⇧T`: bulk creation** (oagc-0pq.5): sheet, default selection, one
   request, review and add.
8. **Settings › Tasks** (oagc-0pq.6): edit, reorder and reset categories.
9. If time remains: a Dock badge or sidebar count of tasks due today.

## Checks before closing an issue

`scripts/gate.sh` (never piped) and `scripts/test-macos.sh test`; CI
green on the push. Record spec amendments when a decision changes. List
decisions in the handoff. Run a review agent over `main..overnight-5`
before the handoff and fix what it confirms.

## Guardrails (in addition to CLAUDE.md)

- The maintainer's app may be running against the real account. Do not
  launch the app against the real account and never start sync outside
  the fakes. Snapshots only via `scripts/snapshot.sh` (scratch data
  directory, demo mailbox, throwaway preferences) with
  `-OpenAGCFakeAgents YES`. Leave a running OpenAGC alone.
- No real agent runs: tests and snapshots use the fake agents. No Claude
  cloud routines.
- Nothing connects to Gmail, IMAP, Google identity endpoints or any real
  account; tests use `FakeProvider` and `provider_gmail::imap_fake`.
- Never read `~/.claude/.credentials.json`, `~/.codex/auth.json`, or the
  Keychain items under service `ai.actual.openagc`.
- Do not delete anything under `~/Library/Application Support/OpenAGC`.
- After the run, compare mtimes of the real `accounts/index.json`,
  `~/Library/Logs/OpenAGC/core.log` and
  `~/Library/Preferences/ai.actual.openagc.plist` against the start.
- Watch disk space (`df -h /System/Volumes/Data`).

## Morning handoff

PR to `main`, closed issues, spec amendments, decisions, anything
half-done with its `bd` notes, CI status, guardrail check results.
