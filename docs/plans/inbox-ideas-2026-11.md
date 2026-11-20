# Inbox ideas — 2026-11

Plan for 11 features and 6 interface improvements that came out of a
review of another mail client's product page on
2026-11-16. One branch and PR per phase, from `main`. Fake agents and the
demo account only.

## Context

A review of another mail client against Kaluta found that Kaluta already has
the hard parts (writing guide, facts, Waiting, calendar holds, routines,
Granola, MCP) but lacks a handful of features and several presentation
patterns. This plan covers all 17 items. Each is designed to fit Kaluta's
rules: no backend, labels rather than hidden state, AI proposes and the
user decides, every AI-written email ends on an editable screen before
Send, and "Kaluta" is the name of the AI in the interface.

Paths are relative to the repository root.

## Decisions (proposed; the maintainer confirms)

1. **⌘K becomes the command palette.** Text that matches no command is sent
   to Kaluta on Return, so "⌘K, type, Return" still asks the agent.
2. **Send later needs Kaluta running.** Gmail has no scheduled-send API and
   there is no backend. A scheduled send that is under 1 hour late goes out
   on launch; later than that, Kaluta asks first. Quitting keeps scheduled
   sends (today quitting releases every held send).
3. **Search by meaning uses Apple's on-device embeddings**
   (`NLContextualEmbedding`), computed in Swift and stored as blobs in the
   account store; ranking is brute-force cosine in Rust. No new Rust model
   dependency, nothing leaves the Mac. Needs an ADR.
4. **Meeting prep reads upcoming meetings from Granola** (its `evt_`
   entries, which `follow_up_meetings` drops today) and from invitations in
   mail. Google Calendar stays free/busy only (ADR 0019 unchanged). It is a
   background job, not a new Routine trigger, so cloud runners are not
   affected.
5. **Compound requests keep the send gate.** One group card approves the
   reversible parts (draft, hold, labels) together; the send still opens
   Review and Send.
6. **Agents may attach only files that are already in the user's mail**
   (by message and attachment id), never a path on disk.
7. **Snippets live in the global store** beside global facts (ADR 0012), so
   every account has them.
8. Shortcuts: Rewrite ⌘J, Send Later ⌥⇧⌘D (⇧⌘L is Star), Remind `b` in the
   thread list (as in Triage).

## Repository rules every phase follows

- **Undo ledger (ADR 0006).** Any change to message or draft state
  (scheduling, rescheduling or cancelling a send, remind, watch, attach to
  a draft, move to Inbox) records its inverted delta in the undo ledger
  *before* the outbox job is enqueued, and the state change, the ledger
  record and the outbox row are committed in one database transaction.
- **Timestamps are `mail_domain::Millis`.** Every time that crosses the
  FFI or is stored (`scheduled_at`, `remind_at`, `until`, watch and brief
  times) is `Millis`; no raw integers or mixed date types in public
  contracts or store functions. Swift converts `Date` to `Millis` at the
  `CoreClient` boundary, as existing calls do.
- **No hand-written parsers or serialisers for domain types.** New domain
  types derive Serde. Free-text time input in the UI is read with the
  system's `NSDataDetector` only (no bespoke grammar) and becomes `Millis`
  before it leaves the view model.
- **Check resolved versions first.** Before writing code or tests in a
  phase, read `Cargo.lock`, the root `Cargo.toml`, `rust-toolchain.toml`
  and `macos/project.yml` for the exact resolved versions of what the work
  uses (rusqlite, `mail-domain`, rmcp, the Swift Testing version shipped
  with the pinned Xcode) and write to those versions.
- **Routine prompt fingerprints.** Where an accepted routine change alters
  the generated prompt (U2), a test checks the fingerprint differs from
  the baseline generation.

## Step 0: record the plan

- Save this plan as `docs/plans/inbox-ideas-2026-11.md` in the repo's plan
  format (What exists, Decisions, phases).
- Create a beads epic and one `oagc-` issue per item below, each citing its
  spec section. Spec changes land as "(Amendment 2026-11-…)" subsections
  of `docs/SPECIFICATION.md`; ADRs 0022 (send later), 0023 (search by
  meaning), 0024 (meeting prep), 0025 (command palette and one command
  layer for the mail window).
- One branch and PR per phase, from `main` (the current branch is
  `forwarding-rules`).

## Phase 1: shared pieces and quick wins (branch `ideas-1-foundation`)

**U6 Key hint footer.** New `KeyHints` component in
`macos/Kaluta/Design/Components.swift` built from `Keycap`
(`Design/Workflow.swift:103`): "↑↓ Select · ↵ Confirm". Use it in
`LabelPicker.swift`, `Tasks/TaskMatcher.swift` and the new pickers below.
Add to `docs/design-system.md`.

**F4 Command palette.**
- New `Features/Palette/`: `PaletteModel` (pure, filter and rank, modelled
  on `LabelPickerModel`) and `PaletteView` (a field over a list, with
  `KeyHints`).
- New `MailCommand` registry: id, title, key label, availability reason,
  action. `App/KalutaApp.swift` menus and the `keyDown` switch at
  `ThreadListView.swift:245` read from it, so menu, list key and palette
  share one implementation (principle 15). `KeyboardShortcutGuide.groups`
  is generated from the registry, which removes the "keep in step" comment.
- Rows: commands, mailboxes and labels, accounts, snippets (F5), and a
  last row "Ask Kaluta: <text>" calling `model.focusAgentPrompt()` with the
  text. Unavailable commands are greyed out with the reason.
- Rebind ⌘K at `KalutaApp.swift:~284`.

**F2 Send later.**
- Migration `00NN_scheduled_sends.sql`: `outbox.scheduled_at` (nullable,
  `Millis`). `mail-store/src/outbox.rs`: `enqueue_scheduled`,
  `scheduled_sends`, `reschedule`; `release_held_sends` and `send_held_now`
  (`kaluta-core/src/compose.rs:307`) skip rows with `scheduled_at`.
- Undo: schedule, reschedule and cancel each write their inverted delta to
  the undo ledger first, in the same transaction as the draft's state
  change and the outbox row (the pattern `send_draft_holding` and
  `cancel_send` use today), so ⌘Z takes a scheduled send back to a draft.
- `Core::send_draft_at(draft_id, when)` beside `send_draft_holding`
  (`compose.rs:357`). `SyncService` already wakes on `next_outbox_retry`.
- Launch check for overdue rows (decision 2).
- UI: a Send Later menu on the composer's Send button and in
  `WorkflowWriteView`, using the time picker from F3. Scheduled mail shows
  in Drafts with a "Sends Tue 9:00" chip; opening it offers Send Now,
  Change Time, Cancel. Cancelling is the existing `cancel_send`.

**F3 Natural-language time picker.**
- New `Features/TimePicker/TimeInput.swift` (pure): the typed text is read
  by `NSDataDetector` (it handles "8am", "in 3 days", "Aug 7"); otherwise
  the text filters the preset rows. No hand-written grammar. The result is
  converted to `mail_domain::Millis` before any core call. Presets come
  from the existing `reminderTimes()` and `checkAgainTimes`
  (`AppModel+Workflows.swift:1376-1409`).
- `TimePickerView`: a field ("Try: 8:00 AM, 3 days, Aug 7"), preset rows,
  `KeyHints`, and an **If no reply** toggle.
- Replaces the numbered remind and check-again choices in Triage and
  Waiting (number keys still pick presets), and brings Remind to the mail
  window on `b` via `core.remindLater`.
- "If no reply" calls `start_waiting` with `until` set, rather than a plain
  reminder, so a reply cancels it (`store::answered`).

## Phase 2: writing (branch `ideas-2-writing`)

**F5 Snippets.**
- Global store: table `snippets(id, name, shortcut, body_html, updated_at)`
  in `data_dir/global/`, new `kaluta-core/src/snippets.rs`, FFI in
  `CoreClient.swift`.
- Settings › Snippets pane (list and editor, like `FactsSettings.swift`).
- Insert: typing `/` at the start of a line in `RichTextEditor` opens a
  filter popover (the `LabelPickerPopover` pattern); also from the palette.
  Placeholders `{first name}` filled from the first recipient.
- "Save as Snippet…" on a selection in the composer.

**F6 Rewrite in place (⌘J).**
- `ComposerAssistant.apply` replaces the whole body today
  (`ComposerAssistant.swift:244`). Add `rewrite(range:instruction:)` that
  sends only the selection (with the draft as context) through the same
  read-only session and replaces that range, keeping attributes outside it.
- Popover at the selection: Improve Writing, Fix Spelling and Grammar,
  Shorter, Longer, and a free field. With no selection it acts on the
  paragraph. Undo is the text view's own undo.
- Recorded through the existing `recordWritingHelp` (ADR 0013).

**U5 Marking Kaluta's drafts.**
- Drafts list rows get a sparkles badge when the draft has an
  `ai_compositions` row (`mail-store/src/compositions.rs`); help text
  "Written by Kaluta, not yet read".
- A "Your voice" chip on the composer banner (`ComposerView.swift:122`,
  `:389`) and `WorkflowWriteView.answersNote` when the writing guide was
  followed; clicking it opens the guide.

## Phase 3: the agent (branch `ideas-3-agent`)

**F1 Compound requests with a receipt.**
- Core: stamp each proposal with the turn it came from (`turn_id` on
  `AgentEvent::ActionProposed`, `agent-api/src/lib.rs:192`, set in
  `agents/approvals.rs:159`).
- New tool `mail_attach_to_draft(draft_id, message_id, attachment_id)`:
  add to `permissions/src/lib.rs` as Reversible, to
  `agent-mcp/src/catalog.rs` (regenerates `docs/mcp.md`), and implement in
  `agents/tools.rs` using the composer's existing draft attachment path.
- Swift: `AgentStore` groups consecutive proposals with one `turn_id` into
  a `ProposalGroupCard`: one row per result (Drafted the reply, Held Thu
  10:00, Attached X.pdf), one Approve for the reversible rows, and Review
  and Send for the draft. Extends `approveAll` (`AgentStore.swift:203`).
- Tests: `agents/tests.rs`, `injection_tests.rs` (an email must not be able
  to make the agent attach from another thread without it showing on the
  card), `AgentPanelTests.swift`.

**F10 Calendar beside the thread.**
- `FreeBusyStrip`: today plus two days as busy blocks, from
  `calendar_free_slots` and `busy()`; no titles. Shown beside the reader
  when the thread asks to meet (the check at `AppModel+Workflows.swift:247`)
  and always available from a toolbar button.
- "Insert Free Times" in the composer writes the next slots as text;
  clicking a slot offers Hold (`calendar_hold`). Also pass `freeTimes` into
  the composer's writing help, as `WorkflowWriter.swift:494` does.

## Phase 4: background and alerts (branch `ideas-4-background`)

**F8 Catch-up line.** Store "last looked" per account; `workflowOverview`
(`kaluta-core/src/workflows.rs:347`) returns new-since and needs-you counts
from existing triage suggestions (no new inference).
`StartScreenView` headline: "18 new since 9:00 · 3 need you". The three
open in Triage.

**F11 Reply-landed alerts.**
- Table `watches(thread_id, set_at)`; "Tell Me When They Reply" in the
  mail window, Waiting and after Send.
- On `CoreEvent::NewMail` for a watched thread from someone else: call
  `settle_answered` at once (today it waits for the waiting job), emit a
  `ReplyLanded` event, and notify even when Kaluta is frontmost.
- `NewMailNotifier` gains an action category: Reply (opens the writer with
  ready answers; the `reply_answers` job takes watched threads first) and
  Remind Me.

**F9 Meeting prep.**
- New job `prep` in `JOBS` (`workflow_jobs.rs:49`, cap 10 a day): for
  meetings starting within the next day, a hidden read-only session
  (`ask_agent_hidden`) writes a brief from local mail: who, last
  exchanges, open threads, what the user owes. Stored in `meeting_briefs`.
- Dashboard card "Next: Raviga call at 10:00 · brief ready" and a
  notification 15 minutes before; the brief opens as a reader page.

**F7 Search by meaning.**
- Uses the `mail-domain` id and `Millis` types at the version resolved in
  `Cargo.lock` (checked first); the embeddings table is derived data, not
  message state, so it has no undo record.
- Table `message_embeddings(message_id, model, vec BLOB)`. Swift
  `EmbeddingIndexer` embeds subject plus the first part of the body, idle
  and on battery-friendly settings, newest first, with progress in the
  sync footer. Core exposes `pending_embeddings` and `store_embeddings`.
- Query: the search field embeds the query in Swift; core returns the
  union of FTS and nearest neighbours, FTS first. A "Related" section
  marks meaning matches. Operators (`from:`, `label:`) still filter.
- Agent tool `mail_search` gains `mode: "meaning"`.
- Attachment text is out of scope for the first version (filenames only,
  as now).

## Phase 5: surfaces (branch `ideas-5-surfaces`)

**U1 Routine chips on rows.** In `ThreadRowView.chips(for:)` (:77) order
routine bucket labels first and draw them in the bucket's colour; show the
chip in All Mail and search results, where the mailbox does not already
say it.

**U2 Correct it once.** Move to Inbox (`AppModel.swift:1260`) on a thread
under a routine's `parent_label` calls the same `routine_proposals::propose`
as Weekly reading's `back_to_inbox` (`reading.rs:372`). The undo notice
gains a one-line field, "Tell Kaluta why", saved as evidence on the
proposal. The move itself stays one undoable change (ledger record and
label change in one transaction). A test checks that accepting the
proposal changes the routine's prompt fingerprint from its baseline.

**U3 One memory page.** "What Kaluta Knows": one list over `FactsStore`
and the guide entries with tabs All, Preferences, People and counts, and a
scope chip on each row (Global, account, or a person's address). Rows open
the existing `FactDetail` and `GuideEntryCard`; nothing moves in storage.

**U4 Routine cards.** `RoutineList` rows (`RoutinesWindow.swift:49`)
become cards: name, one sentence built from the routine's buckets, the
schedule in words (already rendered by `agent-api/src/routines/schedule.rs`)
and the switch.

## Verification

- Every issue: `scripts/gate.sh` and `scripts/test-macos.sh test` (design
  lint, help lint, Swift tests). If tests fail with a full disk, remove
  `target/debug` first.
- Rust unit tests beside each new store and core module (outbox scheduling
  including quit and overdue launch, watches, snippets, embeddings ranking,
  prep job caps, and that each state change and its undo record commit
  together); Swift Testing for the pure models (`PaletteModel`,
  `TimeInput`, proposal grouping), written against the Testing version
  the pinned toolchain resolves (checked in `macos/project.yml` and the
  lock files before writing them).
- New surfaces: light and dark PNGs in `docs/design/` via
  `scripts/snapshot.sh`, and entries in `docs/design-system.md`.
- By hand in the demo mailbox (`-KalutaDemo YES -KalutaFakeAgents YES`,
  fake calendar on): palette, send later across a quit, the time picker,
  a compound request, a reply-landed alert, and a meaning search.
- `MenuTests.swift:61` must stay green for the new shortcuts.
