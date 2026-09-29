# ADR 0008: Tasks live on the Mac; Gmail sees them as a `Task` label

- Status: Accepted
- Date: 2026-09-29
- Amends: ADR 0001 register, **Storage** (migration `0009_tasks`, exclusion
  narrowings)
- Spec: §6.2, §14.8; plan `docs/plans/overnight-2026-09-29.md`

## Context

The maintainer wanted a task-based way through email: Claude suggests what
to do about an email, by when and in which category; accepted tasks go in
a list the user works through, usually by replying. Tasks needed a home,
and the rest of Gmail (web, phone) should show which emails have one. The
Inbox should be able to hide them, like Important Only.

## Decision

- Tasks are stored **per account in the account's store** (`tasks`,
  `task_categories`, `task_meta`), on this Mac only. Threads and messages
  are referenced by Gmail id, not foreign key, so a task outlives its thread
  leaving the local store.
- Gmail sees them only as the account's **`Task` label**, found by name or
  created once and remembered by id. A thread carries it while it has an
  open task; the label comes off with its last open task and goes back on
  when one reopens. These are ordinary label changes through the outbox,
  moved by the task operations' own undo. A `Task` label the user puts on
  by hand, with no task here, is never touched.
- The store gains **exclusion narrowings** (`INBOX+!Label_7`) next to
  inclusion ones, so "Hide Emails with Tasks" combines with Important Only,
  category tabs and filters.
- Categories are a fixed starting set, editable per account.
- Sending a reply started from the task list **asks** whether the task is
  done (amended 2026-09-29 from completing it automatically).

## Consequences

- No new Google scope, no account actions, and no server: the tasks work
  offline and in archive accounts.
- Tasks do not appear on the web or phone, only the label does; notes,
  due days and categories stay on this Mac, and are not synced between Macs.
- Deleting a task never touches its email.
- The task list is a SwiftUI `List` (small, not the 100k-row case of ADR
  0001's thread list); its single-letter keys are caught with an AppKit key
  monitor because the list's table view consumes them first.

## Alternatives considered

- **Google Tasks**: another scope and Google account actions; Tasks has no
  place for categories or the email's context.
- **Only a Gmail label, no local tasks**: no due days, categories, notes or
  done state.
- **Archive on accept**: rejected by the maintainer; the email stays where
  it is.
