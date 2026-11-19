# ADR 0022: Send later schedules the draft, not the outbox

- Status: Accepted
- Date: 2026-11-18
- Amends: spec §6.2 (store), §14.5 (composer), §14.6a (quitting)
- Spec: §14.5 amendment 2026-11-18

## Context

The user wants to write a message now and have it go at a time they
choose. Gmail has no API for scheduled sends, and Kaluta has no server
(spec §0), so whatever sends the message at its time is the app itself.

The first plan put the send in the outbox with a time. Reading the send
path showed three things wrong with that:

- A send is frozen when it is queued, and its optimistic copy appears in
  Sent at once. A message scheduled for Thursday would sit in Sent from
  Monday, dated Monday.
- The outbox gives up on an op three days after it was queued
  (`MAX_RETRY_AGE_MS`). A send scheduled further out would be abandoned
  at its first network error.
- Quitting releases every held send, so each of these needed an
  exception for scheduled ones.

## Decision

1. A scheduled send is a draft with a time: a row in `scheduled_sends`
   (draft id, `send_at`, `set_at`, times in `Millis`), removed with the
   draft. Nothing is queued until the time comes.
2. The draft stays a draft: editable, mirrored to the provider's drafts,
   listed over Drafts with its time. Sending it by hand, or its time
   coming, unschedules it first, so a send taken back with Undo Send is
   an ordinary draft and cannot go again by itself.
3. The app asks the core to send what is due when the soonest time
   comes, at launch and on waking. The core sends each by the ordinary
   send with no Undo Send hold, since no one is there to take it back.
4. **Kaluta must be running.** A draft found more than one hour after
   its time is not sent: the app asks (Send Now, Keep as a Draft, Ask Me
   Later). What a message says may no longer hold a day late.
5. A scheduled draft that cannot be sent (no recipient) becomes an
   ordinary draft again and is reported; it is not retried at every
   check.
6. Scheduling, changing the time and unscheduling are undone from the
   app's undo stack with the time the draft had before, as reminders are
   (they change no mail and nothing at the provider).

## Consequences

- Quitting needs no special case: nothing scheduled is in the outbox.
- A crash between unscheduling and sending leaves a draft that was not
  sent, never one sent twice.
- Only the accounts open in the app are looked at. A scheduled draft in
  an account not opened since launch waits until that account is opened.
- The composer's banner and the Send Later help both say "if Kaluta is
  running then".
- Adds migration 0040; older builds cannot open a store that has it.
