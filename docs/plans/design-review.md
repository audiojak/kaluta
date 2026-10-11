# Design review: focused workflows — 2026-10-11

Kaluta keeps the mail app everyone knows (mailboxes, the thread list, the
reader, the composer, search), and adds a set of **workflows**: focused,
full-window experiences for the jobs people open a mail app to get done.
This plan names those jobs, sets the principles the workflows follow, and
describes the review that measures how each job goes in the app today, as
the baseline the workflow designs are measured against.

Agreed with the maintainer 2026-10-11. The review runs the app with
computer use, on the demo account with fake agents only (see *Method*).

## Tasks

A task is a job with a clear finish. The daily ones are what the
workflows are for; the on-demand ones stay in the mail app.

### Daily: the workflows

| # | Task | Done when | Today |
|---|---|---|---|
| 1 | **Triage what's new**: decide each new conversation (reply, make a task, archive, leave it) | everything new has a decision | in pieces: the Inbox, Important-only, category tabs, task suggestions (§14.8), asking the agent "What's new since yesterday?"; no single pass |
| 2 | **Answer what needs a reply**: a queue of conversations where someone waits on the user; an AI draft in their style (§14.9); edit, send, next | the queue is empty | the agent's "Which need a reply?" results and the composer's writing help; the queue does not outlive the agent conversation |
| 3 | **Work the task list**: gather, decide, usually finish by replying | today's tasks are done | Tasks (§14.8), in the sidebar's Favorites |
| 4 | **Chase what the user is waiting on**: sent mail with no answer after a few days; nudge or drop | every overdue one is nudged or dropped | nothing but the *Follow Up* task category |
| 5 | **Skim the low-priority stream**: newsletters and notifications a routine sorted into cadence labels (§11); read the report, clear in bulk | the batch is read or cleared | routines, their labels and run reports |

Tasks 1–3 are the core daily loop.

### Supervising the AI: also workflows

| # | Task | Done when | Today |
|---|---|---|---|
| 6 | **Check what agents did or want to do**: approvals, drafts agents made, routine runs to keep or undo, agent mailboxes' mail (sign-ups, codes) (§7.9, §10.4, §11.6) | nothing waits on the user | approval cards in the agent column, routine history, separate agent accounts |
| 7 | **Teach it**: decide the proposed rules and facts (§14.10, §14.11) | the queue is empty | Review mode, which already takes the whole window for one job |

### On demand: the mail app

| # | Task | Where |
|---|---|---|
| 8 | Find something, or ask about the mail | search, ⌘K |
| 9 | Write a new message | the composer and its writing help |
| 10 | Clear a mailbox in bulk, once a quarter or so | Clean Up (§14.12) |
| 11 | Set up accounts, agents, routines, the writing guide | Settings, onboarding |

## Principles

From the maintainer (1–8) and agreed additions (9–16).

1. **Works alongside other Gmail clients.** Using Kaluta must never make
   another client harder to use. What a workflow records is Gmail state
   where it can be (labels, as Tasks uses its `Task` label), not hidden
   local state. Counts and queues follow changes made elsewhere, and that
   work counts toward progress rather than looking like a glitch.
2. **The mail app stays; each workflow is its own place.** A workflow is
   a completely focused experience and may look nothing like the mail app
   or the other workflows: its own layout, scale and pace. What stays the
   same is the vocabulary: type roles, controls, act-then-Undo, and the
   same key for the same verb (`e` is archive or done everywhere). This
   is how it sits with the design system's "one way to do each thing".
3. **Obvious gamification.** Where there is a set to get through, a score
   card and a visual reward (an animation) for finishing it.
4. **Large, readable text, styled by what it is.** See 16.
5. **Animated transitions between workflows**, so the mind starts on the
   next job before the screen or the data has arrived. A transition plays
   *while* data loads and never adds waiting: the targets in §1.3 hold.
   This widens the design system's motion rule (0.15–0.25 s) for
   transitions between workflows only.
6. **Voice-ready.** A voice interface comes next; every component and
   control is built for it. See 15.
7. **Say when data is loading.**
8. **Say when an action will take time.** Acknowledge every action at
   once; show progress only past about a second, so quick actions never
   flash a spinner (as the sidebar's "Checking for New Mail" waits two
   seconds); the user can keep working meanwhile.
9. **One thing at a time.** The current item and the few actions that
   finish it; the next item comes by itself (as in Review mode).
10. **Every workflow has an end and can be left anywhere.** A clear
    "done for now". Esc returns to the mail app with the item the user was
    on selected; coming back resumes where they stopped.
11. **Reward finishing; never punish.** No streaks to lose, no shaming red
    counts; rewards can be turned off.
12. **Honest counts.** Figures come from the real mailbox, including what
    changed elsewhere. Progress is never faked.
13. **Act, then Undo, in workflows too.** No confirmation dialogs; undoing
    a decision puts the item back in the queue.
14. **The AI proposes; the user decides; the difference shows.** AI work
    is marked as AI work, with its one line on why. Nothing is sent from
    the user's own accounts without them.
15. **One command layer for keys, voice, buttons and the agent.** Each
    action is a named command with one implementation that all of them
    call. Every control has a short, unique spoken name; nothing is only
    possible by hover, drag or swipe; every workflow can answer "what's
    left?" and "where am I?".
16. **Content, context, controls.** Three tiers of type: content (the mail,
    the decision) in large reading type; context (who, when, why) quieter;
    controls (menus, buttons) smallest and consistent. The thread list's
    12–13 pt roles are for the mail app; workflows get their own, larger
    roles in `TypeRole`.

Every animation and reward has a still or cross-fade version under Reduce
Motion, and every workflow works with VoiceOver.

## Flow

The aim is that the user gets into flow quickly and stays there.

**Getting in.**
- *What's next.* On launch, or one key, the day as a short list with
  estimates ("12 to triage · ~6 min", "4 replies", "2 tasks due");
  Return starts the first.
- *The first item is ready before the user is.* Queues are built in the
  background: the first conversation's text and its AI suggestion or
  draft exist when the workflow opens.
- *Sets of a known size*: "10 of 34", a few minutes each; a clear goal and
  the place for the reward.

**Staying in.**
- *The AI's suggestion is the default.* Each item arrives with an action
  chosen ("Archive", "Reply — draft ready"); Return accepts it.
- *Prepare ahead.* While the user is on item 3, items 4 and 5 are drafted;
  an item that is not ready yet is passed over for one that is, never a
  spinner.
- *Group similar items*: newsletters together, scheduling together, one
  person's mail together, so the mind stays in one mode.
- *Quick wins first*; a hard item is skipped with one key, without penalty
  (to the end of the set, or into a task).
- *Nothing interrupts.* New mail, notifications and badges wait; the end
  of a set says "3 new arrived while you worked". Sidebar and toolbar step
  aside.
- *Hands in one place*: one key per decision, near the home row (one
  short phrase, once voice exists); the mouse is never required.
- *Progress in the corner of the eye*: a bar or ring, not a number to
  read; each action answers at once, with optional sound.

**Keeping momentum.** The end of a set plays its reward and offers the
next workflow ("Next: 4 replies", Return), with its transition: triage
feeds replies, replies feed tasks. No countdowns; an optional time box
("15 minutes") is the user's choice.

**Coming out and back.** A short summary (what was cleared, how long it
took), then the mail app. After an interruption, re-entry shows where the
user was with a one-line recap of the item.

## Method

Each of tasks 1–7 is walked from start to finish in the running app with
computer use, recording for each:

- the steps, as keys and clicks, and where they happen;
- dead ends, and every switch to another window or mode;
- every place the mouse is required;
- every wait over 100 ms, and every place the workflow would block on a
  model call (the fake agents answer at once, so these come from the spec
  and the code);
- time from launch to the first decision;
- how each principle above is met or missed;
- screenshots, light and dark.

**Running it.** `scripts/demo.sh [seconds between arrivals]` after
`scripts/test-macos.sh build`: the workflow demo (spec §18) in a
throwaway data directory, removed when the app quits (`KEEP=dir` keeps
one). Built 2026-10-11 (epic oagc-jors).

**Safety.** The app runs only on the demo account with
`-KalutaFakeAgents YES`, never against the maintainer's real account and
never with real sync (CLAUDE.md). Nothing under
`~/Library/Application Support/Kaluta` or `…/OpenAGC` is touched.

**Output.** `docs/design-review/` holds one findings page per task (the
baseline, the gaps against the principles, and the opportunities) and the
screenshots; the workflow designs follow from it.
