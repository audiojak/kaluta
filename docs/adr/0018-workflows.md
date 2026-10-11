# ADR 0018: Workflows: the start screen is home, and each daily job is a focused place

- Status: Accepted
- Date: 2026-10-11
- Amends: spec §14.3 (the window opens on the start screen, not the
  mail view); §10.4 step 5 (*Approve all* leaves sends out); §14.8
  (a send finishes a task by choice, never by a dialog; tasks from an
  earlier day are carried over); §14.10 (what the user tells Kaluta to
  write is evidence; routine changes are proposed like rules); §11
  (summaries made when the routine sorts; moving sorted mail back to the
  Inbox proposes a routine change)
- Builds on: ADR 0007 (read-only embedded agent sessions), ADR 0008
  (tasks local with a Gmail label), ADR 0011 (writing guide), ADR 0013
  (recording AI compositions)
- Spec: §14.13; plan `docs/plans/workflows.md`; review and decisions
  `docs/plans/design-review.md`, `docs/design-review/`

## Context

The design review of 2026-10-11 walked the seven daily jobs (triage,
replies, tasks, waiting on others, the routine's reading, agents,
teaching) through the app. Every job could be done, but each was
assembled from a mailbox, a composer window, an agent conversation, a
settings window and sheets, none knew what "done" was, and only Review
mode (§14.10) gave one decision at a time with an end. The maintainer
wants the mail app kept for people and for other Gmail clients, and each
job to be a focused experience that gets the user into flow quickly.

## Decision

- **The start screen is home.** The main window opens on "what's next":
  the day's workflows with counts and estimates; the mail app is one key
  (M) away and keeps everything it does today.
- **Seven workflows**, each a full-window place in the main window, the
  way Review mode already is: Triage, Replies, Tasks, Waiting on others,
  Weekly reading, Agents, Teach Kaluta. One item at a time from a set
  fixed on entry; a known count; an end with a score, a short reward and
  the next workflow.
- **One command layer.** Every action in a workflow is a named command
  with one implementation that keys, buttons, VoiceOver and (next) voice
  call; the same verb has the same key in every workflow.
- **Drafts follow the user.** Kaluta writes ahead only where the answer
  is short and predictable (a quick reply, a nudge) and then offers up to
  three different answers; otherwise it asks what the user wants to say
  first. Every AI-written email from the user's own accounts ends on an
  editable screen before Send. Agent mailboxes keep sending without
  approval.
- **State Gmail can see.** What a workflow records about a conversation
  is a label where it can be: `Task` (as now) and a new `Waiting` label.
  Local state is limited to suggestions, the waiting log, summaries and
  workflow progress.
- **Background work is daily and logged.** A start-of-day check decides
  which sent mail expects an answer, keeping a running log so no message
  is checked twice; triage suggestions and reading summaries are made
  ahead, in hidden read-only sessions (ADR 0007), capped per day.
- **Kaluta is the one that learns and acts** (principle 17): the
  interface says Kaluta where it means the AI and names the inference
  backend only where it is chosen or audited, or where mail goes to it.

## Consequences

- A new layer in the app (workflow host, components, command layer,
  rewards, transitions) and in the core (suggestion jobs, the waiting
  log and label, reading summaries, routine proposals, workflow
  progress).
- The mail app is unchanged except where a decision above amends it
  (Approve All, Mark the Task Done?, backend names).
- Inference cost rises: triage suggestions for new mail, the waiting
  check and summaries. All are capped per day, run only while the app is
  open and idle, and use the same agent the user chose.
- Without an agent, workflows still work: no suggestions or ready
  answers, and "Write it yourself" in place of "Write it".
