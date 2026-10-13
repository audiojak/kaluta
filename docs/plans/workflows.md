# Workflows — plan, 2026-10-11

Build the start screen and the seven workflows of spec §14.13 (ADR 0018),
from the design review (`docs/plans/design-review.md`, its Decisions,
and `docs/design-review/`) and the designs on the canvas *Kaluta
Workflows* (https://claude.ai/artifact/RDwYuhs22AcmZWZfg3xYhu). Epic and
issues: see *Issues* below (beads, prefix `oagc-`).

Branch per phase (`workflows-1-foundation`, …) from `main`; PR per
phase. Each issue closes with `scripts/gate.sh` and
`scripts/test-macos.sh`, its screens in the workflow demo
(`scripts/demo.sh`), and snapshots in `docs/design/`.

## Shape of the work

**Foundation first, then one workflow at a time, Triage first.** Every
workflow shares the host, the components, the command layer, the
rewards, the progress store and the job runner; building them once,
then Triage on top, proves the shape before six more use it.

### Phase 1: foundation

- **Workflow host and the start screen as home.** `AppModel.workflow`
  (like `reviewMode`) replaces the split view in `MainWindow.swift`; the
  window opens on the start screen; **M** and *View › Mail* toggle; the
  choice is remembered for the rest of the day. Start screen cards from
  the core's counts and estimates; Return, 1–7.
- **Components and type.** New `TypeRole` roles (display, content,
  context, control) and `Tone.workflow(_)` accents; the header, the
  progress bar, the rail, the decision bar with keycaps, the end screen;
  `docs/design-system.md` and `scripts/design-lint.sh` updated.
- **The command layer.** `WorkflowCommand`: every action named once,
  with its key, spoken name and VoiceOver action; *What's left?* and
  *Where am I?*; the keys table of §14.13 in the Keyboard Shortcuts
  window.
- **Rewards and motion.** The ring and burst; the G major chord (a
  bundled sound, or synthesised with `AVAudioEngine`), silent when the
  system's interface sounds or Kaluta's *Play sounds* are off; the
  start → workflow and item → item transitions; Reduce Motion.
- **Sets and progress in the core.** The query for each workflow's set
  and its count (one FFI call for the start screen, `WorkflowsChanged`
  events); `workflow_progress` (the set fixed on entry, the position,
  the decisions, resume, the day's scores, *Done today*); estimates from
  the user's pace.
- **The job runner.** The daily review's scheduling (§14.10,
  `analysis_run.rs`) generalised: after sync settles, while no workflow
  is open, hidden read-only sessions (ADR 0007), caps per account per
  day, surviving a quit.
- **Writing components.** Up to three answers (1–3) with *Tell Kaluta
  what to say*; *What do you want to say?* with starters and dictation;
  the editable final email with *Change* and quick changes. Directions
  recorded with the composition (ADR 0013) and read by the daily review
  as evidence (compare and glean prompts).
- **Send and finish.** *Send and finish the task* / *Send, keep the task
  open* wherever a reply is written (composer included); the "Mark the
  Task Done?" question goes.
- **The demo.** The workflow demo's agent answers every new prompt
  (triage suggestions, the waiting check, summaries, nudges, routine
  proposals) through the real tools, and its mailbox has what each
  workflow needs.

Already filed and part of this phase: `oagc-px8w` (Review and Send),
`oagc-jurb` (say Kaluta, not the backend).

### Phase 2: Triage

- **Triage suggestions job**: the 30-second rule; per new conversation a
  kind (quick reply, task, archive, leave), seconds, a reason, and up to
  three answers for a quick reply; `triage_suggestions`.
- **The Triage workflow**: the set (new since the last finished set,
  people first, automated grouped), the item, the quick reply in place,
  the task in place (an existing task shown, never a second), arrivals
  waiting for the end, the end.

### Phase 3: Tasks and Replies

- **Tasks**: today's set, carried over first; *What did you decide?*;
  *Send and finish*; *Done*; *Tomorrow*.
- **Replies**: conversations waiting on the user with no task;
  direction first, then the editable email.

### Phase 4: Waiting on others

- **The waiting check**: the start-of-day job, the `waiting_checks` log,
  the `Waiting` label (found or created, through the outbox, as `Task`),
  answers clearing it.
- **The Waiting workflow**: longest first; nudges written ahead; let go;
  check again.

### Phase 5: Weekly reading and Teach Kaluta

- **Summaries at sort time** and **due buckets** (Friday by default, a
  day per bucket).
- **The Weekly reading workflow**: the report, buckets, clear (read,
  label kept), undo the run.
- **Routine proposals** from *Back to the Inbox*.
- **Teach Kaluta**: Review mode widened to rules, facts and routine
  changes in one queue.

### Phase 6: Agents

- **The Agents workflow**: proposed sends (after `oagc-px8w`), codes and
  sign-ups in agent mailboxes (codes notify), routine runs; what agent
  mailboxes sent on their own.

## Testing

Core: each set's query, the jobs' prompts and parsing (fake agent
answers), the waiting log (never twice), labels through the outbox,
progress and resume, estimates. App: XCTest for stores and commands
(every key maps to one command; every command has a spoken name);
UI tests for start → Triage → end; snapshots of each screen light and
dark. The workflow demo exercises every screen with computer use.

## Out of scope

Voice input itself (the command layer is built for it); calendar; a
workflow for agent mailboxes' own inboxes beyond codes and sign-ups;
syncing workflow progress between Macs.

## Issues

Epic **oagc-k6o8**. Related, filed earlier: `oagc-px8w` (Review and Send; the Agents workflow waits on it) and `oagc-jurb` (say Kaluta, not the backend).

| Phase | Issue | What | Depends on |
|---|---|---|---|
| 1 | oagc-k6o8.1 | Sets, counts and progress in the core | |
| 1 | oagc-k6o8.2 | Job runner for background jobs | |
| 1 | oagc-k6o8.3 | Components, type roles and accents | |
| 1 | oagc-k6o8.4 | Command layer | |
| 1 | oagc-k6o8.5 | Rewards and motion | .3 |
| 1 | oagc-k6o8.6 | Host and the start screen as home | .1, .3, .4 |
| 1 | oagc-k6o8.7 | Writing components; directions as evidence | .3, .4 |
| 1 | oagc-k6o8.8 | Send and finish, or keep the task open | |
| 1 | oagc-k6o8.9 | The workflow demo | |
| 2 | oagc-k6o8.10 | Triage suggestions job (30-second rule) | .2 |
| 2 | oagc-k6o8.11 | Triage workflow | .5, .6, .7, .8, .10 |
| 3 | oagc-k6o8.12 | Tasks workflow | .6, .7, .8 |
| 3 | oagc-k6o8.13 | Replies workflow | .6, .7 |
| 4 | oagc-k6o8.14 | Waiting check, its log and the Waiting label | .2 |
| 4 | oagc-k6o8.15 | Waiting on others workflow | .6, .7, .14 |
| 5 | oagc-k6o8.16 | Summaries at sort time; due buckets | .2 |
| 5 | oagc-k6o8.17 | Back to the Inbox proposes a routine change | |
| 5 | oagc-k6o8.18 | Weekly reading workflow | .6, .16, .17 |
| 5 | oagc-k6o8.19 | Teach Kaluta workflow | .6, .17 |
| 6 | oagc-k6o8.20 | Agents workflow | .6, oagc-px8w |
