# ADR 0019: Calendar access is its own consent, after sign-in

- Status: Accepted
- Date: 2026-10-29
- Amends: spec §1.2 (no calendar), §7.7 (scopes), §10.2 (tools)
- Spec: §7.7, §10.2, §14.9, §14.11; plan `docs/plans/overnight-2026-10-28.md`

## Context

Replies to "when can we talk?" are a large share of what the user
answers. Kaluta could write them only with times the user typed, or a
calendar link from their facts. Offering real free times needs the
user's calendar; holding them while the other person decides needs to
write to it. Kaluta signs in to Gmail with `gmail.modify` (or full mail
access for IMAP) and `openid profile`, all asked for at sign-in.

## Decision

- **Not at sign-in.** Calendar access is asked for only when the user
  chooses *Connect Google Calendar…* in Settings › Accounts, for the open
  Google account, by running consent again with the account's own
  client and `include_granted_scopes=true`. Existing accounts are never
  sent back to Google for something they did not ask for.
- **The least scopes.** `calendar.freebusy` to read when the user is busy
  (never what the events are), and `calendar.events.owned` to make and
  delete holds on calendars the user owns. Not `calendar` or
  `calendar.readonly`.
- **What the token grants is what is recorded**, in the account's store,
  each time sign-in completes: signing in again without the calendar
  scopes leaves the calendar unconnected rather than failing later.
- **Slots, not events.** Free slots are computed in the core from
  free/busy blocks; agents get labels and times, never titles,
  attendees or descriptions.
- **Holds invite no one.** A hold is a tentative, private event with no
  attendees and no reminders, at most four hours, within 60 days, one per
  call, recorded so the user can release it. Inviting the other person
  is sending and stays out.
- **The calendar link stays.** Without a connected calendar, a *Calendar
  link* fact (cal.com, Calendly, a Google appointment page) is offered,
  and without that Kaluta asks; it never invents times. No booking
  service's API is used.

## Consequences

- Google lists the calendar scopes on the consent screen as sensitive;
  publishing the app's OAuth client with them needs Google's verification
  for those scopes, as it does for the mail scopes.
- A user who revokes Kaluta's access in their Google account revokes the
  whole token, mail included; *Stop Using the Calendar* only forgets the
  calendar in Kaluta.
- Other calendars (iCloud, Outlook) are not covered.
