# ADR 0013: Every AI composition is recorded, then compared with what was sent

- Status: Accepted
- Date: 2026-10-05
- Amends: ADR 0001 register, **Storage** (new per-account tables) and
  **Agents** (a daily background review)
- Builds on: ADR 0004 (per-account store), ADR 0006 (exact undo),
  ADR 0007 (read-only embedded sessions), ADR 0011 (writing guide)
- Spec: §14.10; plan `docs/plans/analysis.md`

## Context

The writing guide (ADR 0011) is learned once from sent mail and then
changes only when the user edits it. The best evidence of where it is
wrong is what the user changes in an AI draft before sending it, but
nothing kept the AI's text: writing help writes into the draft body,
autosave overwrites it, agent tools save straight into the draft, and the
draft row is deleted once the send is accepted.

## Decision

- **Record every AI composition** with its full text, in the account's
  store (`ai_compositions`): writing help, the agent column's draft tools
  and routines. A later AI text for the same draft (a rewrite, another
  audience, `update_draft`) replaces the record's text; the last AI text
  before the user's edits is what is compared. The record keeps the
  instruction, recipients, thread, guide version and audience, and copies
  the draft's rfc822 Message-ID when it is sent, so the record outlives
  the draft. A discarded draft marks its record discarded.
- **Hidden until the account has finished one writing-guide learning
  run.** Before that nothing is recorded. Archived-mailbox accounts never
  record.
- **Match, then compare, once a day.** A daily review per account (only
  while the app is open, first chance after the day's first sync settles)
  matches records with the message the user sent, compares the pair in a
  hidden read-only agent session (ADR 0007, mail fenced, quotes verified)
  and proposes guide changes. Nothing changes until the user accepts, and
  every acceptance is undoable (ADR 0006).
- **Retention:** the full AI text and the sent text are kept 30 days after
  the record is reviewed (Settings: 7, 30 or 90 days). After that the
  record keeps only its distance, status and the proposals it supported.
  An unmatched record expires 14 days after it was made.
- **Local only.** Records never leave the Mac except as the pairs the
  daily review sends to the user's own agent CLI, the same disclosure as
  learning; Analysis says so before the first review.

## Consequences

- Each account store holds up to a month of AI drafts beside the mail they
  became; the retention purge keeps it bounded.
- The review costs agent turns every day; a cap (50 pairs a day by
  default) bounds it, and the rest waits for the next day.
- A sent message matched by thread or recipient rather than by Message-ID
  may not be the draft's descendant; matches with too little word overlap
  are dropped rather than compared.

## Alternatives considered

- **Diff the draft at send time**: misses drafts edited in Gmail, sent
  from another client or rewritten from scratch, and still needs the AI
  text kept somewhere.
- **Keep AI texts forever**: grows without bound and keeps copies of mail
  the user may have deleted.
- **Compare immediately on send**: one agent turn per message, and single
  edits are noise; the daily batch needs two or three examples before it
  proposes anything.
