# Plan: IMAP-first sync

Status: implemented 2026-09-28 (steps 1–6; spec §7.3, §7.4 and §14.7a amended). Accepted with the maintainer's decisions (end of file).
Supersedes the "hybrid, IMAP for bulk only" design of spec §7.4
(amendment 2026-09-26) once accepted.

## The rule

IMAP is the default transport for Gmail accounts, not an option. The Gmail
API is used only:

1. **when it is faster** for that job (measured, not assumed);
2. **when there is no other way** to get the data (categories, draft ids,
   label colours, sending with Gmail's threading); or
3. **when IMAP fails** (IMAP disabled by the user or a Workspace admin,
   authentication refused, the daily bandwidth budget spent, a connection
   error), per operation, with a circuit breaker so one failure does not
   flip the whole account.

Every sync operation records which transport served it, so the choice is
visible in the sync status and the log, and testable.

## Why now

- The API's unit quota makes bulk download slow (20 units per
  `messages.get`; the first real mailbox saw less than the documented quota).
  IMAP has no unit quota, only bandwidth (~2,500 MB/day) and 15 connections.
- The current split already causes gaps: messages are *listed* over the API
  but *fetched* over IMAP from `[Gmail]/All Mail`, which has no drafts, spam
  or trash. Drafts were missing entirely until today's `drafts.list` sync
  (oagc-6rf); Spam and Trash are still never downloaded (listing passes
  `includeSpamTrash=false`).
- Scope is not the obstacle it looked like: `gmail.modify` (today's default)
  is already a restricted scope (§7.3), so asking for
  `https://mail.google.com/` does not change Google's verification or CASA
  assessment. What changes is the consent screen's wording ("read, compose,
  send and permanently delete all your email") and what a leaked token could
  do.

## Who does what

| Job | Transport | Why |
| --- | --- | --- |
| List mail in the window, by tier (unread Inbox first) | **IMAP** `UID SEARCH X-GM-RAW "…"` on `[Gmail]/All Mail` | No quota; Gmail's search syntax works over IMAP |
| Headers, snippets, bodies, attachments | **IMAP** (as today's backfill) | Bulk, no quota |
| Spam and Trash | **IMAP** `[Gmail]/Spam`, `[Gmail]/Trash` | Not in All Mail; never synced today |
| Drafts | **IMAP** `[Gmail]/Drafts` for content; **API** `drafts.list` for draft ids | Editing and saving in place need the draft id (no other way) |
| New mail, at once | **IMAP IDLE** on All Mail (and Inbox) | Push instead of a 30 s poll |
| Label and read-state changes made elsewhere | **API** `history.list` (2 units) | Faster: Gmail IMAP has no change log (no `CONDSTORE`/`QRESYNC`, spec §7.4), so the IMAP way is re-reading flags for the whole window |
| Categories (Primary, Promotions…) | **API** | Not exposed over IMAP |
| Labels, colours, visibility | **API** `labels.list` | Colours and ids only there |
| Archive, label, read, star, junk, trash (outbox) | **API** `batchModify` / `trash` | One call for up to 1,000 messages; exact per-message diffs for undo (§14.6a); our own changes are recognised in history |
| Send, save and delete drafts | **API** | Threading and draft ids; SMTP would need a separate path for nothing gained |
| Server search for older mail | **IMAP** `X-GM-RAW`, API as fallback | No quota |

The API stays on for the jobs in its column, so an account always holds
both an IMAP session and API tokens from the same sign-in.

## Design

1. **Sign-in asks for `mail.google.com` by default.** The Settings toggle
   "Download faster over IMAP" goes away; Settings shows the transport in
   use and why ("IMAP", or "Gmail API: IMAP is turned off in Gmail's
   settings"). Accounts signed in with `gmail.modify` keep working on the
   API (decision 2: no banner).
2. **A transport layer in `mail-sync`** replaces `BackfillSource` with a
   `Transport` that each engine job asks for by capability (`list`,
   `fetch_headers`, `fetch_bodies`, `watch`, `changes`, …). It picks IMAP or
   API per the table, falls back per operation, and trips a per-account
   breaker after repeated IMAP failures (retry after 15 minutes, or at once
   when the user asks for a refresh).
3. **IMAP listing** replaces REST listing phases: the same phases expressed
   as `X-GM-RAW` queries (`in:inbox is:unread`, `newer_than:30d`, …),
   resolved to `X-GM-MSGID`s in UID order, fed to the same queue. The id map
   (UID ↔ `X-GM-MSGID`) the backfill already keeps is reused.
4. **Folders beyond All Mail.** Spam, Trash and Drafts are selected and
   synced like All Mail, limited to the window; their messages carry
   `SPAM` / `TRASH` / `DRAFT` from the folder, since `X-GM-LABELS` omits
   them.
5. **IDLE.** One long-lived connection per account in IDLE on All Mail
   (re-issued every 25 minutes, as Gmail drops idle sessions at ~29); an
   `EXISTS` triggers an incremental round at once. The 30 s / 5 min poll
   stays as a backstop for label changes via history.
6. **Connections.** At most 4 per account (1 IDLE, up to 3 fetch), well
   under Gmail's 15; accounts share nothing.
7. **Budget.** The daily bandwidth budget stays (yield to the API at
   2,000 MB), now per account and shown in Settings.
8. **Spec.** §7.3 (scopes), §7.4 (sync) and §14 Settings are amended; the
   2026-09-26 "hybrid" amendment is marked superseded.

## Measure before deciding "faster"

A benchmark on the fakes cannot say whether IMAP or the API is faster
against Gmail. The Sync Debugger (decision 4) runs a comparison on demand:
listing 10,000 ids, fetching 500 headers, 500 bodies, and reading the
changes of the last day, each both ways, reporting timings and counts
only. The table above is the expected outcome; the numbers decide.

## Order of work (issues)

1. Transport layer with per-operation fallback and the breaker; today's
   behaviour expressed through it (no change in what is used).
2. Sign-in with `mail.google.com` by default; Settings shows the transport;
   the re-sign-in banner for existing accounts.
3. Listing over IMAP (`X-GM-RAW` phases).
4. Spam, Trash and Drafts folders over IMAP.
5. IDLE push for new mail.
6. The Sync Debugger with the comparison, and the spec amendments.
7. Remove the IMAP opt-in and the REST-only listing path once 1–6 are in.

Each step is tested against the fake IMAP server and `FakeProvider`: the
fake IMAP server gains folders, `X-GM-RAW`, `IDLE` and failure injection
(refused auth, dropped connection, budget exhausted) so every fallback is
exercised. Nothing in automation connects to Gmail.

## Decisions (maintainer, 2026-09-28)

1. **Consent wording:** accepted. New accounts sign in with
   `mail.google.com`.
2. **Existing accounts:** switched silently. The only existing account
   already granted `mail.google.com` (it backfills over IMAP), so it moves
   to IMAP-first with no prompt; the re-sign-in banner is dropped. An
   account without the scope stays on the API until its next sign-in.
3. **Fallback visibility:** a quiet note in the sidebar's sync footer
   ("Using the Gmail API: IMAP is turned off in Gmail's settings").
4. **Sync debugger:** built and kept, not a one-off. A Sync Debugger window
   (Window menu) shows per account the transport serving each job and why,
   the breaker's state, IMAP capabilities and bandwidth used today, recent
   operations with timings, and runs the IMAP-vs-API comparison on demand.
   It reads and times only; it changes no mail.
