# Overnight run — 2026-10-08

Branch `overnight-7`, created from `main` after PR #13 was merged, in the
main checkout (no second worktree: the disk has ~15 GiB free and `target/`
is 56 GiB).
Commit and push after every closed issue. Open a PR to `main` in the
morning; never push to `main` overnight.

Two goals: a Clean Up window for clearing a mailbox in bulk (feature 2),
and agent mailboxes grouped under one service account, with AgentMail as
the second service (feature 1).

## Order (maintainer, 2026-10-08)

1. Clean Up, core (feature 2, issues C1–C4).
2. Service accounts and AgentMail (feature 1, issues 1–5).
3. Clean Up, the rest (feature 2, issues C5–C7).
4. *(Optional)* AgentMail WebSocket push (feature 1, issue 6).

Each closed issue is committed and pushed before the next starts, so a
run that stops early leaves whole features, not halves.

## Feature 1: service accounts and AgentMail (epic oagc-uys)

### Decisions (maintainer, 2026-10-07)

- **One key per service account.** A *service account* is what the
  service calls an organisation (AgentMail) or an account (Primitive). It
  holds the API key (one Keychain item), the verified human email, the
  plan and limits, and own domains. Each *agent* in it is an OpenAGC
  account as today: its own store, writing guide, facts, undo and send
  mode (ADR 0004 holds).
- **Primitive gets several agents per service account too.** Primitive
  refuses to verify a second account with the same email (`email_in_use`),
  so the model of one Primitive account per mailbox could never verify a
  second mailbox with the user's email. Its managed subdomain receives at
  any local part and it sends from any verified domain, so each agent is
  a local part (`scout@jade-emu.primitive.email`, `writer@…`) on one
  account, and on its own domains once added.
- **Display:** agents stay separate accounts. The account switcher groups
  them under their service account ("AgentMail · you@example.com"). A
  service-account settings pane holds what is shared: verification, plan
  and limits, *Copy API Key*, *Rotate Key*, domains. No combined "All
  Agents" inbox tonight.
- **AgentMail asks for the user's email up front.** The create sheet
  chooses the service first; for AgentMail it asks for the human email
  (prefilled from the open account), because without it the inbox is
  receive-only and a lost key cannot be recovered. The code is sent at
  sign-up; *Fill Code from <address>* works as for Primitive.
- **AgentMail labels sync both ways**: read state, archive and user labels
  go to AgentMail through the outbox (undoable). Deleting stays local:
  nothing the user does deletes or trashes mail at the service (ADR 0014).
- **Polling first**, against a wiremock fake; WebSocket push is the last,
  optional step.

### What the docs say (read 2026-10-07; docs.agentmail.to, docs.primitive.dev)

AgentMail, base `https://api.agentmail.to`, `Authorization: Bearer`:
- Sign-up `POST /v0/agent/sign-up {username, human_email}` → `api_key`,
  `inbox_id`. Repeating it with the same email returns the same
  organisation with a **rotated key** (and resends an expired code): never
  call it for a service account that already exists. Verify
  `POST /v0/agent/verify {otp_code}` (6 digits, 24 h, 10 tries).
- Before verifying: sends only to the human (403 `message_rejected`);
  creating API keys is refused (403 `missing_permission`). Free plan: 3
  inboxes, 3,000 mails a month; a new claimed inbox may send to 3
  recipients in its first hour, 5 the first day, 10 the first week.
- More agents: `POST /v0/inboxes {username, domain?, display_name}` →
  `inbox_id`, `email`.
- Read: `GET /v0/inboxes/{id}/messages` (`limit`, `page_token`, `after`,
  `before`, `labels`, `ascending`; items carry `labels`, `timestamp`,
  `updated_at`, `to`, `cc`, …), `…/messages/{mid}/raw` for MIME.
- Labels: `PATCH …/messages/{mid} {add_labels, remove_labels}`. `unread`
  and `read` are labels by convention; `trash` hides a message; there is
  no archive label and no inbox label. `GET /v0/inboxes/{id}/events` lists
  `label.added` / `label.removed`, newest first, paged.
- Send: `POST …/messages/send` (and `…/{mid}/reply`) takes JSON (`to`,
  `cc`, `bcc`, `reply_to`, `subject`, `text`, `html`, `attachments` base64,
  `labels`, `headers`), 6 MB per request, **no idempotency key**.
- Inbox-scoped API keys exist (`POST /v0/inboxes/{id}/api-keys`) once
  verified.

Primitive: received rows carry `to_email`; sent rows carry `from_header`.
The managed subdomain "can receive at any local-part".

### Design

- **Service accounts in the core.** `services/<id>/service.json`
  (service, human email, verified, plan cache, domains); the Keychain
  item becomes `mailbox.api_key.<service id>`. An agent's `agent.json`
  gains `service_account` and, for AgentMail, `inbox_id`.
  **Migration:** an `agent.json` without `service_account` becomes a
  service account whose id is the agent's account id, so the existing
  Keychain item keeps its name and nothing is re-keyed. Removing the
  last agent of a service account deletes the record and the key.
- **UniFFI:** `list_service_accounts`, `add_agent(service_account, name)`,
  and plan, verification, *Copy API Key*, *Rotate Key* and domains move to
  the service account (the per-agent calls stay as thin wrappers until
  the app no longer uses them).
- **Primitive, several agents:** adding an agent makes no API call; its
  address is its name as a local part on the service account's managed
  subdomain (or a verified own domain), unique within the service account.
  Each agent's `PrimitiveProvider` keeps the rows addressed to it
  (`to_email`, and To/Cc of the raw message) and the sent rows from it
  (`from_header`). Mail to a local part no agent has goes to the service
  account's first agent, marked "to <address>" (default; revisit).
  Own domains move from the agent to the service account.
- **New crate `provider-agentmail`** (deps `mail-domain`, `mail-mime`,
  `provider-api`; add to `xtask` check-deps): `AgentMailService`
  (`MailboxService`: sign-up with the human email, verify, plan, add an
  inbox) and `AgentMailProvider` (`MailProvider`):
  - Listing per inbox, paged; messages fetched as raw MIME through the
    existing MIME path.
  - Labels: `unread` ↔ `UNREAD`; a received message without the app's
    `archived` label is in `INBOX`; sent ↔ `SENT`; other labels map by
    name. Changes go out as `PATCH add_labels/remove_labels`. Delete and
    trash stay local.
  - Changes: poll `messages?after=<newest timestamp>` for arrivals and the
    events list down to the last seen `event_id` for label changes.
  - Send: MIME → JSON (`to`/`cc`/`bcc`, text, html, base64 attachments,
    `In-Reply-To`/`References` and an `X-OpenAGC-Outbox-Id` in
    `headers`). No idempotency key: after a timeout or unknown result, the
    outbox looks for that header among sent messages before it retries,
    and never sends twice.
  - Errors: AgentMail's `{name, code, message, fix}` body through
    `HttpClient`'s per-provider hook; `message_rejected` before verifying
    is said in those words.
- **App:**
  - *Create an Agent Mailbox…* chooses the service first. If a service
    account for it exists, the sheet offers *Add to <service account>*
    (name only: no terms, sign-up or code) or a new service account.
  - AgentMail's path asks for the human email before *Agree and Create*.
  - Account switcher groups agents under their service account; the
    service-account settings pane; agent settings keep name, address and
    send mode. *Copy API Key* on AgentMail offers an inbox-scoped key when
    verified, and says the organisation key reaches every agent in it.
  - The agent prompt and composer state AgentMail's limits.
- **Spec and ADR:** ADR 0015 *Agent mailboxes belong to service accounts*
  (amends ADR 0014's per-mailbox key and one-account-per-mailbox);
  spec §7.9 (service accounts, AgentMail, adding an agent), §12 (the key
  name), §0 Secrets.

### Issues, in order

1. **ADR 0015 and spec §7.9/§12** (oagc-uys.11).
2. **Core: service accounts** (oagc-uys.12) — record, migration of existing agent
   mailboxes, Keychain key per service account, removal of the last
   agent, FFI. Tests: migration keeps the key name; two agents share a
   key; removing one keeps the key.
3. **Primitive: several agents per service account** (oagc-uys.13) — add an agent,
   split inbound by recipient and sent by sender, unmatched mail to the
   first agent, domains on the service account. Wiremock tests with two
   agents on one fake account.
4. **`provider-agentmail`: service and provider** (oagc-uys.6, retitled)
   — sign-up, verify, add inbox, list, raw, labels both ways, events,
   send with the duplicate-send guard, error mapping. Wiremock fake of
   every endpoint used; check-deps; `FakeMailboxService` learns AgentMail.
5. **App: create sheet, grouping, service-account settings** (oagc-uys.14) — service
   choice, human email for AgentMail, *Add to <service account>*, the
   switcher groups, the settings pane, limits text. Tests, snapshots light
   and dark, design-system entries.
6. *(Optional)* **AgentMail WebSocket push** (oagc-uys.15) as the account's push
   source, falling back to polling.

### Left for the maintainer to check by hand (real accounts; never in automation)

- Primitive: a send `from` a second local part goes out as that address;
  the email listing's `to_email` is the envelope recipient.
- AgentMail: the labels a received and a sent message carry (`received`,
  `sent`?); whether `unread` is set on arrival; the error for a taken
  username and for the fourth inbox on the free plan.

## Feature 2: Clean Up (epic oagc-merk)

A Mailstrom-style bulk cleaner: see the mail grouped by sender, subject,
time, size and so on, tick the groups, and archive, move, trash or mark as
spam thousands of messages at once, with undo. Used once a quarter or
year, so it is not in the main window.

### Decisions (maintainer, 2026-10-08)

- **Its own window**, like Routines: `Window("Clean Up", id: "cleanup")`,
  opened from *Mailbox › Clean Up Mailbox…* and *Settings › Accounts ›
  Clean Up…*; it cleans the open account. A one-time `TipCard` in the
  Inbox suggests it when the Inbox holds more than 1,000 messages. Nothing
  is added to the main sidebar.
- **Inbox by default, with an *All Mail* toggle.** The progress card
  tracks the Inbox.
- **Opening Clean Up loads every header.** If the account's sync window is
  not *Everything*, opening the window sets it to *Everything* (headers
  only; the body window is unchanged) and shows the header load's
  progress. The window says the setting changed; Settings › Accounts
  shows it. On an account without IMAP, where headers are not cheap, the
  window states the message count and the time it will take and asks
  before starting (the one exception, recorded in the handoff).
- **Mailing lists from new mail only.** `List-Id`, `List-Unsubscribe` and
  `List-Unsubscribe-Post` are stored from now on, with no refetch of old
  mail; the Mailing Lists view fills as mail arrives and says so when
  empty.
- **Social and Promotions from Gmail's categories**, grouped by sender
  domain (`CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`); no brand list.
  The second view is called *Promotions*, not Shopping.
- **Not tonight:** Block, Chill and Expire (they need standing local
  rules, a plan of their own, perhaps as built-in routines), and Forward.
- **Messages, not threads.** Groups count messages and actions change
  those messages only: archiving the "Amazon" group leaves the replies of
  real people in a mixed thread where they are.

### The views (the window's left column)

| View | Groups by | Source |
|---|---|---|
| Sender | `from_email`; the most used name as the title, the others as "aka …" | `messages` |
| People I've Emailed | senders the user has written to | `contacts.sent_count > 0` |
| Subject | identical subject (as stored, `Re:` kept) | `messages.subject` |
| Mailing Lists | `List-Id` (name and domain) | new `messages.list_id` |
| Time | Today, Yesterday, This Week, Last Week, then months | `messages.date` |
| Social | sender domain within `CATEGORY_SOCIAL` | `message_labels` |
| Promotions | sender domain within `CATEGORY_PROMOTIONS` | `message_labels` |
| Size | Tiny <1 KB, Small 1–10 KB, Medium 10–100 KB, Large 100 KB–1 MB, Extra Large 1–10 MB, Jumbo >10 MB | `size_estimate` |

### Design

- **Store (migration `0018_cleanup`):** indexes for grouping (`from_email`,
  `date`, `size_estimate`); `list_id`, `list_unsubscribe`,
  `list_unsubscribe_post` columns on `messages`, written by the header
  and full fetch paths (IMAP header block, REST `metadataHeaders`, MIME);
  `inbox_history` (day, count at midnight) and `cleanup_meta` (the Inbox
  count when Clean Up was first opened, the progress baseline).
  *(as built, C1: §14.11 was taken by Facts, so Clean Up is §14.12, and
  the store is §6.2, not §8. A `list_name` column holds List-Id's phrase.
  REST fetches only `format=full`, which carries every header, so there
  was no `metadataHeaders` list to extend. The indexes are covering
  (`from_email, from_name, date` and so on). The 20,000-message apply
  bench belongs with the apply, C2.)*
- **Core API (FFI):** `cleanup_groups(view, scope, filter)` → rows (key,
  title, aka, count), by count; `cleanup_messages(view, scope, keys,
  offset, limit)` → message rows (sender, subject, date, size);
  `cleanup_count(view, scope, keys)`; `cleanup_apply(view, scope, keys,
  action)` with `Archive`, `Move(label)`, `Trash`, `Spam` → one undo
  entry (ADR 0006, per-message diffs) and `LabelOp`s through the outbox
  in chunks of 1,000 (`batchModify`'s limit), reporting progress as
  events; `cleanup_progress()` → baseline, midnight, received today,
  removed today, now, and the daily counts for the sparkline. The set of
  messages is resolved when the action runs, so a group that grew since
  it was shown is acted on as it is now; the undo entry records exactly
  which messages changed.
- **Performance:** groups for a 100,000-message mailbox answer in under
  200 ms and a 20,000-message apply writes locally in under 2 s (Rust
  benches over synthetic stores, recorded in `docs/performance.md`).
- **Window (SwiftUI, AppKit for the long lists):**
  - Left: the views as a source list; under them the **progress card**
    (Inbox Zero %, sparkline, At Midnight, Received Today, Removed Today,
    Now) using the design system's card and numeric styles.
  - Middle: a filter field ("Type a sender…"), then the groups: checkbox,
    title, aka line in the secondary style, count. Multiple selection as
    in the mail lists; Space ticks.
  - Right: the messages of the ticked groups in the calm row style
    (sender, subject, date; size in the Size view), virtualized rather
    than paged; the header says "*N* messages in *M* groups".
  - Toolbar: *Inbox / All Mail* (segmented), *Archive* (`e`), *Move…*
    (label picker, as in mail), *Trash* (`⌫`), *Spam* (`!`), and later
    *Unsubscribe*. Acting applies to every message in the ticked groups;
    the undo notice says "Archived 813 messages from Amazon" with *Undo*
    (⌘Z). Long applies show progress in the toolbar.
  - Empty states (`ContentUnavailableView`) for a view with no groups,
    Mailing Lists before any list mail arrives, and no ticked groups.
- **Spec:** a new §14.12 *Clean Up*; §6.2 gains the columns and tables;
  §7.4 notes that Clean Up widens the sync window. No ADR needed unless
  message-level actions conflict with an existing one (check ADR 0006).

### Issues, in order

C1. **Spec §14.12 and the store** (oagc-merk.1) — migration `0018_cleanup`, the list
    headers on every fetch path, grouping queries and indexes, benches.
C2. **Core: groups, messages and apply** (oagc-merk.2) — the FFI above, message-level
    `LabelOp`s, one undo entry per apply, progress events. Tests against
    `FakeProvider`, including undo of a 2,500-message trash (three
    batches) and a group that changes between showing and acting.
    *(as built: the calls take the account id; Trash and Spam go out
    as `batchModify` label changes, undo and redo too, rather than a
    trash call per message; progress is `OutboxStatus` after each
    batch; `cleanup_progress` moves to C5.)*
C3. **The Clean Up window** (oagc-merk.3) — the menu item and Settings button, the
    three columns, the five ready views (Sender, People I've Emailed,
    Subject, Time, Size), toolbar actions, keys, undo, empty states.
    Tests, snapshots light and dark (demo account with bulk demo mail),
    design-system entries, the Keyboard Shortcuts window.
C4. **Loading every header on open** (oagc-merk.4) — set the window to *Everything*,
    show the header load, the no-IMAP confirmation. Tests with the IMAP
    fake.

*(Then feature 1, then:)*

C5. **The progress card** (oagc-merk.5) — `inbox_history` written at the first sync
    after midnight and on open, baseline, sparkline, the four numbers.
C6. **Social and Promotions views** (oagc-merk.6).
C7. **Mailing Lists view and Unsubscribe** (oagc-merk.7) — group by `List-Id`;
    *Unsubscribe* for groups whose messages carry `List-Unsubscribe`:
    the one-click POST (RFC 8058) after a confirmation that names the
    sender and the URL's host, or a mailto that opens the composer
    filled in for the user to send. Never automatic; never an agent's.
    The Inbox tip for large inboxes lands here too.

## Checks before closing an issue

`scripts/gate.sh` (never piped; `if scripts/gate.sh >log 2>&1; then …`)
and `scripts/test-macos.sh test`; CI green on the push. Record spec
amendments when a decision changes. List decisions in the handoff. Run a
review agent over `main..overnight-7` before the handoff and fix what it
confirms.

## Guardrails (in addition to CLAUDE.md)

- Nothing calls a real Primitive or AgentMail endpoint: every sign-up
  creates a real account. Wiremock fakes only.
- Clean Up acts on fakes and scratch stores only. Unsubscribe is never
  exercised against a real URL: tests use a local wiremock server.
- The maintainer's app may be running against the real account. Do not
  launch the app against it and never start sync outside the fakes.
  Snapshots only via `scripts/snapshot.sh` with `-OpenAGCFakeAgents YES`.
  Leave a running OpenAGC alone.
- No real agent runs; no Claude cloud routines.
- Never read `~/.claude/.credentials.json`, `~/.codex/auth.json`, or the
  Keychain items under service `ai.actual.openagc`.
- Do not delete anything under `~/Library/Application Support/OpenAGC`.
  The service-account migration is tested on scratch data directories
  only.
- After the run, compare mtimes of the real `accounts/index.json`,
  `~/Library/Logs/OpenAGC/core.log` and
  `~/Library/Preferences/ai.actual.openagc.plist` against the start.
- Watch disk space (`df -h /System/Volumes/Data`). Below 5 GiB free,
  remove `target/debug/incremental` (rebuilt on demand) before anything
  else; never delete outside `target/`.

## Morning handoff

PR to `main`, closed issues, spec amendments, decisions (including any
defaults taken, such as where unmatched Primitive mail goes), anything
half-done with its `bd` notes, CI status, guardrail check results, and the
hand-check list above.
