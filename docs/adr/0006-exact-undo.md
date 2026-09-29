# ADR 0006: Act, then offer an exact undo

- Status: Accepted
- Date: 2026-09-27 (extended 2026-09-28, 2026-09-29)
- Amends: ADR 0001 register, **Storage** (new `undo_actions` table) and the
  outbox's ordering (spec §7.4)
- Spec: §14.6a; design system "Act, then offer Undo"

## Context

Mail actions (archive, trash, labels, read, star) are reversible, and
confirmation dialogs for them slow every action to protect against the
rare mistake. An undo that simply inverts the action is wrong when the
thread was already partly in the target state (undoing "Archive" must not
move a thread that was already archived into the Inbox), and wrong again if
Gmail changed the thread in between.

## Decision

- Actions happen at once and show an undo notice; ⌘Z works on the last
  50 actions per account after the notice is gone. Confirm only what cannot
  be undone.
- The core records each user action in the account's store as
  **per-message diffs**: the labels it actually added and removed. Undo
  applies exactly the inverse (and redo the original) through the outbox,
  in the token's own account. An action that changed nothing returns no
  token.
- The outbox runs **strictly in order**, so an undo never reaches Gmail
  before the action it reverses.
- **Undo Send** holds a send in the outbox for a delay (default 10 s); only
  a send not yet handed to Gmail can be taken back. Quitting sends held
  messages at once.
- Agent and routine actions are not on the user's stack: they have the
  activity log and a run's Undo.
- Non-mail changes (tasks, ADR 0008) register their own undo and redo steps
  on the same per-account stack.

## Consequences

- No confirmation dialogs for everyday actions; undo is predictable even
  after sync changed the thread.
- A failed op holds back the ops after it until it succeeds or is given up
  on; held sends are the only exception.
- Some destructive actions still have neither confirmation nor undo
  (discarding a draft, deleting leftover mail data, deleting a routine);
  tracked as oagc-068.3.

## Alternatives considered

- **Confirm every action**: slow, and people learn to click through.
- **Invert the action** instead of recording diffs: wrong for partly-applied
  actions and after concurrent changes.
