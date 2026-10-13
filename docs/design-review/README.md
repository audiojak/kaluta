# Design review: the baseline — 2026-10-11

How each daily task in `docs/plans/design-review.md` goes in Kaluta today,
walked from start to finish in the running app with computer use, on the
workflow demo (`scripts/demo.sh`: demo mailbox dated from launch, mail
arriving, fake agents that use the real tools). Screenshots are in `img/`,
light appearance, 1200×760 window.

| Task | Page |
|---|---|
| 1. Triage what's new | [1-triage.md](1-triage.md) |
| 2. Answer what needs a reply | [2-replies.md](2-replies.md) |
| 3. Work the task list | [3-tasks.md](3-tasks.md) |
| 4. Chase what the user is waiting on | [4-waiting.md](4-waiting.md) |
| 5. Skim the low-priority stream | [5-stream.md](5-stream.md) |
| 6. Check what agents did or want to do | [6-agents.md](6-agents.md) |
| 7. Teach it | [7-teach.md](7-teach.md) |

**Limits of this baseline.** The fake agents answer at once, so no model
latency was felt; where a workflow would wait on a model, the pages say
so from the code. Under background control the Return key did not reach
text fields or Review mode, so Return paths are unverified (single keys
such as `e`, `r`, `t` worked). Some pure-SwiftUI content can capture
blank (CLAUDE.md); where a screenshot showed an empty pane that may be
that, the page says *unconfirmed*.

## The short version

The parts are good; the jobs are not yet anyone's. Every daily task can
be done, but each is assembled by the user from a mailbox, a separate
composer window, an agent conversation, a settings window or a sheet,
and none knows what "done" is except Review mode (task 7), which is
already the shape the workflows should take: the whole window, one
decision at a time, a count, keys on screen, evidence beside the
decision, and an end ("All Decided — 2 added to your writing guide, 1
left out").

## Findings across tasks, most serious first

1. **A crash.** Switching to the agent mailbox from *Kaluta › Accounts*
   ended the app with exit status 133 (SIGTRAP), with no crash report
   and nothing on stderr. It did not happen again under `lldb` with the
   same data. Needs its own investigation (task 6).
2. **Quit did nothing while "Your Writing Guide Is Ready to Review" was
   up.** *Kaluta › Quit* left the app running behind the sheet (task 1).
3. **Getting in costs four interruptions.** Launch shows the writing-guide
   modal, then the Categories tip, then an Important Only tip, then an
   Ask the Agent tip, before the first email can be decided (task 1).
   Flow: "getting in" fails at the first step.
4. **Nothing says what the job is or how much is left.** The Inbox mixes
   today's mail with weeks-old unread mail; "99 unread · Important only"
   sits over a list of four; Sent says "67 unread". No view counts the
   new mail, the replies owed or what is left (tasks 1, 2, 4). Principle
   12 (honest counts) and the flow goal of a known-size set.
5. **A decision spans windows and steps.** Replying opens a separate
   composer window; sending leaves the conversation in the Inbox; it does
   not complete the conversation's task unless the reply was started from
   the task list (the link, `answeringTaskID`, exists only there). So
   "reply and be done" is reply, send, return to the main window, archive,
   and open Tasks to mark it done (tasks 1, 2, 3). Principles 9 and 13.
6. **The agent column crowds the work.** It opens as a fourth column and
   squeezes the reader to a strip about three words wide; its transcript
   (a row per tool call, eighteen "Read a thread" lines for one question)
   pushes the answer into a box one row tall; it stays open, with an
   earlier conversation, when the user moves to Tasks or Sent (task 2).
7. **Agent work is not reviewable where it happens.** A draft the agent
   writes is not in the conversation and not in Drafts (Drafts lists only
   drafts Gmail has; the agent's draft is a local row until it syncs, and
   the demo has no Gmail); approvals exist only inside the transcript of
   the conversation that asked; the activity log is *Settings ›
   Permissions › Show Activity…*, per account, a raw table (tasks 2, 6).
8. **A confirmation where Undo belongs.** "Mark the Task Done?" is a
   dialog after send, against the design system's act-then-Undo (and
   principle 13); and it is offered only from the task list (task 3).
9. **Finishing is silent.** Archiving the last of something, completing
   a task, or deciding the last rule ends with a small notice or a plain
   page; no score, no reward, no next step: Review mode's end does not
   offer Facts, which has four proposals waiting (tasks 3, 7). Principles
   3, 10, 11 and the flow goal "finishing leads to the next thing".
10. **Text is mail-app sized.** Rows are 12–13 pt, the reader's body about
    15 pt; only Review mode's statement (`TypeRole.display`, about 22 pt)
    is the size principle 4 asks for. Context (dates, chips, "seen in 2
    messages") and controls are the same weight as content in most rows.
11. **Small breaks.** The undo notice is clipped by the window's bottom
    edge in Tasks (task 3); in Sent and the agent's waiting list, mail the
    user started shows "Me" rather than who owes the answer (task 4); after
    a relaunch the agent bar kept reading "Looking for Claude…" although
    the agent was ready; relaunch
    after the crash restored the Routines window alone, without the mail
    window (task 6).

## What to keep

- **Archive advances.** `e` archives and selects the next conversation,
  with "Archived 1 conversation — Undo ⌘Z": the best flow moment in the
  app outside Review mode.
- **Single keys** (`e`, `r`, `t`, `j`/`k`) for every common verb, already
  shared by the mail list and the task list.
- **Review mode** as the template: full window, one item, "0 of 3",
  Accept/Edit/Reject with keys shown, evidence (AI draft beside what was
  sent, edits marked), a collecting-evidence group, an end state with
  Undo.
- **The task list's shape**: Overdue, Today, This Week, No Date; the
  overdue day in the caution colour; "6 open · 3 due".
- **Claude's task suggestion** (`t`): a filled sheet with the category,
  the due day and Claude's one line on why.
- **Writing help in the composer**: one line of instruction, the text
  written in place, "Written by Claude, following your writing guide.
  Read it before sending." with Undo.
- **The approval card**: what will be sent and to whom, Review, Reject,
  Approve; declining tells the agent, and the transcript says so.

## Against the principles

| # | Principle | Today | Where |
|---|---|---|---|
| 1 | Works alongside Gmail | Mostly: tasks use a `Task` label, the routine uses labels. Agent drafts are invisible until Gmail has them. | 2, 3, 5 |
| 2 | Each workflow its own place | Only Review mode. Everything else is the mail app plus panels, windows and sheets. | all |
| 3 | Gamification | None: no score, no reward. | 1, 3, 7 |
| 4 | Large, readable text | Only Review mode. | all |
| 5 | Animated transitions | Not between jobs; the jobs are not places yet. | — |
| 6 | Voice-ready | Every common verb has a key and a menu item, a good base; nothing answers "what's left?" and the agent cannot act on tasks. | 1, 3 |
| 7 | Say when loading | The sidebar's sync line does; the agent column shows a spinner and a Stop button. | 2 |
| 8 | Say when slow | Not measured: the fakes are instant. | — |
| 9 | One thing at a time | Review mode; archive's advance. Elsewhere, lists. | 1, 7 |
| 10 | An end; leave anywhere | Review mode only. | 7 |
| 11 | Reward, never punish | No punishment, no reward. | — |
| 12 | Honest counts | Counts describe mailboxes, not the job. | 1, 4 |
| 13 | Act, then Undo | Yes for mail and tasks; "Mark the Task Done?" is a dialog. | 3 |
| 14 | AI proposes, user decides | Approval cards, "Written by Claude", Claude's why on tasks; agent drafts hard to find. | 2, 3, 6 |
| 15 | One command layer | Keys and menus share commands; the agent's tools do not cover tasks, and no command answers "where am I?". | 3, 6 |
| 16 | Content, context, controls | Not distinguished outside Review mode. | all |

## Flow, measured

| Task | Before the first decision | Per item, today |
|---|---|---|
| 1 Triage | 4 dismissals | archive: 1 key; task: `t` + Add Task; reply: `r`, write, Send, back to main window, `e` |
| 2 Replies | ask the agent (type + send), scroll its column | click a result, `r`, write, Send, back, next result by mouse |
| 3 Tasks | open Tasks | select, `r`, write, Send, answer the dialog, select the next |
| 4 Waiting | ask the agent (no view) | click, `r`, write a nudge, Send |
| 5 Stream | expand Marked Important, pick a label | read one by one; Select All from the menu; no summary |
| 6 Agents | approvals: find the conversation; mailbox: *Kaluta › Accounts › name*; log: four levels into Settings | — |
| 7 Teach | open Writing Guide, Review | one key or button each, auto-advance |
