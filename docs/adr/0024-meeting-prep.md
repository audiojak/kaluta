# ADR 0024: Meeting prep reads upcoming meetings from Granola

- Status: Accepted
- Date: 2026-11-18
- Spec: §14.14 amendment 2026-11-18

## Context

Before a meeting the user wants to know who they are meeting and where
things stand: the last exchanges, what is open, what they owe. Kaluta has
the mail; it needs to know which meetings are coming.

Google Calendar is connected for free/busy only (ADR 0019): Kaluta never
reads event titles or attendees there, and this does not change that.
Granola, when connected (§14.14), already lists calendar entries for
meetings with no note yet, under `evt_` ids; the follow-ups job drops
them.

## Decision

1. A background job, `prep` (cap 10 a day per account), asks Granola for
   today and tomorrow, keeps the `evt_` entries starting within the next
   24 hours, and stores them in `meeting_briefs` (migration 0043).
2. For each meeting without a brief, it reads the user's latest messages
   with each person there (from, to or cc; up to five each), fences them
   as data, and asks the analysis agent, in a hidden read-only session,
   for at most eight short lines. A moved meeting keeps its brief; one
   whose people changed is written again.
3. The Dashboard shows the next meeting with a brief ("Next: Pricing call
   with Reese at 10:00 · brief ready"), which opens to the brief, and a
   notification with the brief comes 15 minutes before.
4. It is a background job, not a Routine trigger, so cloud runners are
   unaffected.

## Consequences

- Without Granola there is no meeting prep. Invitations in mail are not
  read for it yet.
- Briefs say only what the mail shows; a person with no mail gets "No
  mail with them" in the prompt.
- Briefs older than a week after their meeting are deleted.
