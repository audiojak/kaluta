# Overnight run — 2026-10-28

Branch `overnight-2026-10-28`, created from `main` after PR #50 was
merged, in the main checkout. Commit and push after every closed issue.
Open a PR to `main` in the morning; never push to `main` overnight.
Fake agents and the demo account only; the real account is never
launched or synced from automation.

Two goals: whole answers ready for every conversation that needs a
reply, so the user picks one and sends (feature 1); and Kaluta offering
times for a meeting from the user's calendar, or a scheduling link
(feature 2).

## Order

1. Feature 1, issues 1–4: the core job, then the workflows, then the
   cost and freshness rules, then the spec.
2. Feature 2, issue 5 (the scheduling link: an hour, no new access).
3. Feature 2, issues 6–9: Google Calendar read (free slots), the
   writing step that offers times, holds, then settings and the spec.
4. *(Optional)* issue 10: holds and slots on a workflow card.

Each closed issue is committed and pushed before the next starts, so a
run that stops early leaves whole features, not halves.

## Feature 1: whole answers for every reply (epic oagc-d04c)

### What exists (read 2026-10-28)

The triage-suggestions job (`crates/kaluta-core/src/triage.rs`,
spec §14.13 *Jobs*) runs as new mail arrives, in batches of ten
conversations per step, and asks the agent once per batch for a kind
(reply, task, archive, leave), seconds, a reason, and, **only for a
quick reply of 30 seconds or less**, up to three answers, each a
one-line label and the whole email. A reply over 30 seconds, or with
no answers, becomes a task or is left in the Inbox (`parse`). Answers
are stored in `triage_suggestions.answers_json` for the conversation's
latest message and dropped when a newer message arrives. Only Triage
loads them (`AppModel+Workflows.swift:140`): the reply screen opens on
the answers (1–3) with *None of these? Tell Kaluta what to say…*.
Replies and Tasks open on *What do you want to say?* with the email's
own starters (Yes, No, its choices), which Kaluta writes from at the
moment they are chosen, one agent turn each (PR #49).

So the gap is: answers are written only for the quick ones, by the
triage prompt in passing (not by the writer with the writing guide and
facts), and Replies never sees them.

### Decisions (proposed; the maintainer confirms before the run)

- **Needs a reply** is its own judgement, separate from *quick*. The
  triage prompt answers `needs_reply: true|false` for every
  conversation written by a person to the user (not a newsletter, a
  receipt, an automated notice: `category` empty) whose latest message
  is not the user's own and asks or expects something. Triage's kinds
  stay as they are; *quick* keeps the 30-second rule for the bar's
  suggestion.
- **Options come from the email, then from Kaluta.** The options for a
  conversation are the starters the email gives (`WritingText.starters`:
  Yes and No for a yes-or-no question, each side of an "or"), else the
  triage prompt's own up to three `answers` labels for any conversation
  that needs a reply (not only quick ones). At most three.
- **Each option is written by the writer, not the triage prompt.** A
  second job, *Reply answers*, takes each needs-reply conversation with
  options and asks the agent, in a hidden read-only session as the
  writer does (`ComposerAssistant.prompt` with the writing guide,
  `facts_lookup`, the thread), for one whole email per option, the
  option as the instruction ("Yes"), batched one conversation per turn
  with all its options in one answer (`{"answers": [{label, text}]}`).
  When Kaluta would have to ask a question (the questions JSON, spec
  §14.9), the conversation gets **no ready answers** for that option:
  the user's own turn asks them. Nothing is invented.
- **Stored with the suggestion**, in `triage_suggestions.answers_json`
  as today, with `answers_by: "writer"` so the card can tell them from
  the triage prompt's quick answers; dropped when a newer message
  arrives (as now), and written again by the job.
- **Shown in Triage and Replies** on the reply screen as the answers
  screen is today (1–3 pick a whole answer, which lands on the editable
  screen; `/` tells Kaluta instead; Esc back). Tasks' replies too when
  the task's conversation has them. The decision bar's *Reply* button
  says *Reply · 3 answers ready*. Nothing changes on the card itself.
- **Cost.** A conversation is written for once per latest message. The
  job runs in the same slot as triage suggestions (as new mail arrives,
  after the suggestions for that batch), newest conversations first,
  Inbox only, at most **40 conversations a day** per account (a
  `workflow_job_days` cap as the others), with *Run Now* in Settings ›
  Analysis counting against it. An answer is at most 1,200 characters
  per option in the prompt's cap. No answers for conversations older
  than seven days.
- **Freshness.** Sending any reply in the conversation, or a newer
  message, drops its answers (the message-id rule as today). The
  writing guide changing does not rewrite them (they were written with
  the guide of the day; the review will see the edits).

### Issues, in order

1. **Core: needs-reply and options in triage suggestions**
   (oagc-d04c.1). `needs_reply` and `options` (labels) in the
   triage prompt and `parse`; `TriageSuggestion.needs_reply`;
   `triage_suggestions.needs_reply` column (migration 0034, listed
   last, the order test guards it). Tests: a person's question needs a
   reply and gets options; a newsletter does not; the user's own latest
   message does not; a quick reply keeps its answers and seconds.
2. **Core: the Reply answers job** (oagc-d04c.2).
   `reply_answers.rs`: the batch (needs-reply, options, no answers yet,
   newest first, Inbox, seven days), the writer prompt per conversation
   with its options, `parse` (answers by label, questions JSON means
   none), the store (`answers_json`, `answers_by`), the daily cap, the
   job in `workflow_jobs` after triage suggestions and in *Run Now*.
   Fake agent writes an answer per option (and asks for "facts about
   me" as it does). Tests: three options give three whole emails; a
   question gives none for that option and the others stay; the cap
   stops the job; a newer message drops them; the writing guide is in
   the prompt.
   *(as built: options come from the triage prompt, which is told to use
   the email's own choices, Yes and No for a yes-or-no question, else the
   likely answers; Swift's `WritingText.starters` stays on the words
   screen. The writer's prompt is the core's own (`reply_answers.rs`),
   the same rules as the composer's: guide, `facts_lookup`, never invent,
   a `missing` list. `answers_by` is `triage`, `writer`, or `none` when
   the writer wrote nothing (the triage prompt's quick answers stay and
   it is not asked again). Workflow jobs have no Run Now, so none was
   added.)*
3. **App: answers in Replies and Tasks, and the bar** (oagc-d04c.3).
   Replies and Tasks load the suggestion for the current item as Triage
   does; `startWriting` offers the writer's answers for any kind; the
   bar's *Reply · n answers ready*; the answers screen says *Written
   with your guide* under its heading when `answers_by` is the writer.
   Snapshot: Replies with three answers ready. Tests: Replies opens on
   the answers; picking one lands on the editable screen with the
   whole email; Esc goes back; a conversation with none opens on the
   words as today.
   *(as built: the bar's Reply says "Reply · 2 answers ready"; the
   answers screen's note says "Written ahead with your writing guide"
   for the writer's. A task's reply gets them only when the task
   answers the conversation's latest message, which they were written
   for. No snapshot: the self-snapshot has no workflow option and the
   screen was locked overnight; the app test drives the same path.)*
4. **Spec §14.13** (oagc-d04c.4): *Jobs* table (the new row and
   the cap), the reply screen in Triage, Replies and Tasks, the
   decisions above; §14.9 (a question means no ready answer).

## Feature 2: offering times, from the calendar or a link (epic oagc-awri)

### What exists (read 2026-10-28)

No calendar anywhere (spec §1 lists it out of the MVP). The facts
store has an *Availability* category with starter labels *Time zone*,
*Usual hours*, *Calendar link*, *Where I usually am*, *Away or travel
dates* (spec §14.11, `facts.rs:263`), and writing sessions have
`facts_lookup`, so a *Calendar link* fact already reaches the writer
when it looks. The Gmail OAuth flow (`provider-gmail/src/oauth.rs`)
asks for `gmail.modify` (or full mail with IMAP) plus `openid profile`,
with `prompt=consent`; scopes are fixed at sign-in. The tool catalog
(`permissions::Tool`, `agent-mcp/src/catalog.rs`, spec §10.2) has
mail tools only, each with a risk tier (ReadOnly, Reversible,
External) that decides approval.

### Decisions (proposed; the maintainer confirms before the run)

- **The scheduling link comes first and costs no new access.** A
  *Calendar link* fact (cal.com, Calendly, Google appointment page) is
  what Kaluta offers when the email asks to meet and the user has one:
  the writing prompt says so, and the composer's and workflows' *Facts*
  starter asks for it once. No cal.com API tonight: the link is the
  integration (their API needs a paid plan for most of what we'd want,
  and the link is what people send).
- **Google Calendar is read first, write second.** Free slots come from
  the Calendar API's `freebusy` query over the user's primary calendar
  (and the calendars they pick), scope
  `https://www.googleapis.com/auth/calendar.freebusy` (read the
  docs tonight for the exact least scope; `calendar.readonly` if
  freebusy alone is not granted). Holds are tentative events on the
  primary calendar with no attendees, scope
  `https://www.googleapis.com/auth/calendar.events.owned` (or
  `calendar.events`), never invitations: inviting the other person is
  sending, and stays a later feature.
- **Access is asked for when first needed, per account.** Settings ›
  Accounts › *Calendar* has *Connect Google Calendar*, which runs the
  OAuth flow again for that account with the calendar scopes added
  (`include_granted_scopes=true`, so the token keeps mail). The new
  token replaces the account's Keychain item. Until connected, Kaluta
  offers the link if there is one and otherwise asks the user for
  times (a §14.9 question with no options). Nothing asks for calendar
  access at sign-in.
- **Two tools, two tiers.** `calendar_free_slots` (ReadOnly): a range
  and a duration, the user's time zone and usual hours (facts, else the
  Mac's time zone and 9–17), returns up to eight slots as "Thu Oct 30,
  2–3 pm"; the agent's hidden sessions may call it. `calendar_hold`
  (Reversible): makes a tentative *Hold: <subject>* event for a slot,
  returns its id; undoable (deleted) and refused outside the next 60
  days or over 4 hours. Both go through the mail account's own token.
- **What the user sees.** On the editable screen a reply that offers
  times shows them under the email as chips, *Hold these* (⇧⌘H) makes
  the holds (one undo for the set), and the sent reply records which
  slots it offered (`compositions`, for the review). The review can
  later propose a *Usual hours* fact from what the user keeps picking;
  not tonight.
- **No free/busy in the prompt by default.** The agent calls the tool
  when the email is about meeting; the triage summary never does. The
  tool's answer is slots only, never event titles: the agent does not
  see what the user is doing.

### Issues, in order

5. **Scheduling link** (oagc-awri.1). `ComposerAssistant.prompt` and
   the workflow writer's prompt: when the email asks to meet, offer the
   user's *Calendar link* fact if `facts_lookup` finds one, as a plain
   URL on its own line; the missing-facts question for it has the
   category `availability`, label *Calendar link*. Fake agent offers
   the link when told "meet". Tests: a reply to "when can we talk?"
   with the fact carries the link; without it Kaluta asks for the link
   or times (one question, no options). Spec §14.9.
6. **Crate `provider-gcal`** (oagc-awri.2): `freebusy` query and
   `events.insert`/`delete` over `reqwest`, the token from the
   account's Gmail credentials, errors through `HttpClient`'s hook, a
   wiremock fake; `check-deps` (only `provider-api` and `mail-domain`).
   Slots from busy blocks: the range, the duration, the hours, 15-minute
   grid, at most eight, none in the past, none that start within two
   hours. Tests against the fake: busy blocks leave the right gaps; an
   all-day event blocks the day; a hold is made tentative and deleted.
7. **Core and OAuth: connect a calendar** (oagc-awri.3): the
   re-consent flow with calendar scopes added
   (`include_granted_scopes`), stored on the account (`calendar_json`:
   connected at, calendars chosen); `Core::calendar_free_slots`,
   `calendar_hold`, `calendar_release`; the tools in
   `permissions::Tool` (ReadOnly, Reversible), the catalog, the MCP
   names (`calendar_free_slots`, `calendar_hold`), the undo entry.
   Tests: a session without the calendar connected gets a clear
   refusal (`not_connected`); a hold is reversible; the injection suite
   gains "block my whole week" (refused: over 4 hours, and one hold per
   call).
8. **Writing: offer times** (oagc-awri.4). The writing prompt: when
   the email asks to meet and the calendar is connected, call
   `calendar_free_slots` for the next two weeks and offer two or three
   times in the reply as a short list, in the user's time zone, each
   also in the other person's if the thread says it; the editable
   screen reads the times back (`OfferedSlots`: a parser for the
   agent's list, with the slot ids in a trailing JSON line the screen
   strips) and shows them as chips; *Hold these* (⇧⌘H) calls
   `calendar_hold` per slot, one undo. Fake agent offers two slots from
   the fake calendar. Tests: the prompt asks for slots only when the
   email is about meeting; the screen shows chips for the offered
   times; holding makes the events; undo deletes them; a sent reply
   records the slots.
9. **Settings and spec** (oagc-awri.5): Settings › Accounts ›
   *Calendar* (Connect, which calendars, Disconnect), the Keys list
   gains ⇧⌘H; spec §7.7 (scopes), §10.2 (the two tools), §14.9 (times
   and the link), §14.11 (*Calendar link* and *Usual hours* in use),
   §1 (calendar no longer out of scope); ADR 0018 *Calendar access is
   its own consent, after sign-in*.
10. *(Optional)* **Slots on the card** (oagc-awri.6): a Replies or
    Triage card whose email asks to meet shows *Kaluta can offer: Thu
    2 pm, Fri 10 am* under the summary before the user replies, from
    the free slots, and the answers (feature 1) include them.

## Not tonight

- Inviting the other person (an event with attendees): that is a send.
- Reading event titles into prompts, or any calendar other than Google.
- The cal.com API (bookings, availability): the link covers it.
- Rewriting ready answers when the writing guide changes.
