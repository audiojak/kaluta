# ADR 0002: IMAP-first sync for Gmail accounts

- Status: Accepted
- Date: 2026-09-28
- Amends: ADR 0001 register, **Gmail** ("hand-written `reqwest` client,
  `history.list` polling") and **OAuth** (scopes)
- Spec: §7.2, §7.3, §7.4 (amendments 2026-09-25, 2026-09-26, 2026-09-28),
  §14.7a; plan `docs/plans/imap-first-sync.md`

## Context

The register chose the Gmail REST API for everything. The first real
mailboxes showed why that cannot hold:

- `messages.get` costs 20 units whatever the format, and the per-user quota
  was below the documented 6,000 units/min. A 43,000-message account needed
  2.4 hours; ~10 rate-limit responses a minute appeared even at 5,500
  units/min (2026-09-25: the limiter became adaptive and a per-account sync
  window was added, default six months).
- Gmail's IMAP endpoint has no unit quota, only bandwidth (~2,500 MB/day)
  and 15 connections, and cheap header fetches.
- A first "hybrid" design (2026-09-26: IMAP for bulk backfill only, opt-in
  per account) was built, but the API still served listing, new mail,
  Spam/Trash/Drafts and search, each slower than IMAP could do them.

## Decision

IMAP is the default transport for Gmail accounts. The API serves a job only
when it is measured to be faster, when there is no other way to get the
data, or when IMAP fails.

- IMAP: listing the window (`X-GM-RAW`), headers and bodies (API above
  2 MB), Spam/Trash/Drafts via special-use folders, new mail via `IDLE`
  (re-issued every 25 min; the 30 s / 5 min poll stays as backstop), Inbox
  categories via `X-GM-RAW "category:…"`, and server search.
- API: changes made elsewhere (`history.list`; Gmail IMAP has no change
  log), draft ids, label colours, and every write through the outbox
  (exact undo, threading, draft ids).
- Each operation falls back to the API on its own. Three consecutive IMAP
  failures open a breaker for 15 minutes; a refused login or the day's
  2,000 MB budget make IMAP unavailable without counting. Every operation
  is recorded and shown in the Sync Debugger.
- Sign-in asks for `https://mail.google.com/` plus `gmail.modify`, `openid`
  and `profile`. Existing accounts with the scope moved silently; others
  stay on the API until their next sign-in.

## Consequences

- Large mailboxes download in minutes instead of hours; new mail arrives
  by push instead of polling.
- The consent screen asks for full mail access. Verification and CASA are
  unchanged (`gmail.modify` was already restricted), but the wording is
  broader.
- Two transports to keep correct. Mitigated by one parse path
  (`mail_mime::parse` for both), an in-process fake IMAP server for tests
  (nothing connects to Gmail), and the operation record.
- Gmail's IMAP ids join the API's through `X-GM-MSGID`/`X-GM-THRID`; the
  API ids stay the source of truth.

## Alternatives considered

- **API only, tuned** (the 2026-09-25 adaptive limiter and sync window):
  still hours for "everything"; kept as the fallback.
- **Hybrid, IMAP for backfill only** (2026-09-26): superseded; it left the
  slow jobs on the API.
- **`users.watch` push**: needs a Cloud Pub/Sub topic, i.e. a server; IMAP
  `IDLE` needs none.
