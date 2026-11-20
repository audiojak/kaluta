# UX review: the mail window and the Dashboard — 2026-11-20

The review planned in `docs/plans/ux-review-2026-11.md`: ten everyday
jobs examined in the mail window and the Dashboard's workflows, five
arrangements of the mail window mocked up, and three directions for a
design system that is Kaluta's own. Nothing in the app was changed.

| Part | Where |
|---|---|
| 1. The jobs | the ten pages below |
| 2. Mail window layouts | [layouts.md](layouts.md) and `mockups/layout-*.html` |
| 3. A design system of Kaluta's own | [design-system.md](design-system.md) and `mockups/direction-*.html` |

| Job | Page |
|---|---|
| 1. Open the app and find out what needs attention | [01-morning.md](01-morning.md) |
| 2. Triage what is new | [02-triage.md](02-triage.md) |
| 3. Answer what needs a reply | [03-replies.md](03-replies.md) |
| 4. Find one conversation and act on it | [04-find.md](04-find.md) |
| 5. Write a new message | [05-compose.md](05-compose.md) |
| 6. Work today's tasks; make a task | [06-tasks.md](06-tasks.md) |
| 7. Check what is being waited on | [07-waiting.md](07-waiting.md) |
| 8. See and correct what Kaluta knows | [08-knowledge.md](08-knowledge.md) |
| 9. Read, change and correct a routine | [09-rules.md](09-rules.md) |
| 10. Leave mid-job and come back | [10-leave-return.md](10-leave-return.md) |

## How it was done, and its limits

**This is an inspection, not a walk-through.** The plan was to drive
the demo with computer use, as in October. Permission to control the
Debug app was declined on the night, so:

- **Screens** were captured by the app's own snapshot mode
  (`scripts/snapshot.sh`) on the demo mailbox with fake agents, light
  and dark, at 1200 points wide and one at 820 (`img/`).
- **Steps, keys and states** were read from the source, with file and
  line, not pressed. Where a finding rests on the source alone the
  page says "from the source, not run".
- **The Dashboard, the sidebar and the Routines window could not be
  pictured.** They capture blank without screen recording (the known
  limit in `CLAUDE.md`). The Dashboard was reviewed from its view tree
  and source.
- Dark captures show the toolbar's glass as white shapes, and the
  workflow captures are dimmed overall. Both are taken to be capture
  artefacts and are **unconfirmed**; no finding rests on them.
- No timing was measured: fake agents answer at once and nothing was
  pressed.
- Reference patterns are from public documentation only.

The two things an inspection cannot give are how it feels at speed and
what happens on a real mailbox. A half-hour walk through the demo by
the maintainer, with this list in hand, would confirm or strike most
findings.

## The short version

The workflows are good and are Kaluta's own; the mail window is
neither yet. October's review said "the parts are good; the jobs are
not yet anyone's". The workflows have fixed that for five jobs: one
thing at a time, a key named on every action, Undo and no questions, a
count, an end. The mail window, where everything else happens, is still
the platform's mail app with Kaluta attached at the side: its reader is
the first thing to give way, its assistant goes by another product's
name, and the same reply ends differently depending on where it was
sent from.

Three recommendations follow from that:

1. **Fix the eight major findings** below; three are plain defects.
2. **Layout:** give the reader its room, make the sidebar optional,
   and offer the Inbox grouped by what is needed
   ([layouts.md](layouts.md)).
3. **Design system:** start with the words (Kaluta by name, one name
   per place, one voice), then move type and colour towards the
   *Paper* direction, keeping the keycaps and workflow colours
   ([design-system.md](design-system.md)).

## Findings, most serious first

Severity: 4 blocks the job, 3 major, 2 minor, 1 cosmetic. No finding
was rated 4.

### Major (3)

| | Finding | Where | Rests on |
|---|---|---|---|
| F1 | **The reader gives way first.** About 250 points wide with Kaluta's column open at 1200, about 200 at 820; it has no minimum width. | mail window | captures; `MainWindow.swift:136–171` |
| F2 | **Starting a workflow from ⌘K in the mail window shows nothing.** The session starts but the window stays on mail. | palette | source, not run |
| F3 | **⌘0 is bound to two commands** (*Window › Mail*, *View › Dashboard*). | menus | source, not run |
| F4 | **One place, three names, and a button that says a fourth.** Workflows' back button reads "Mail" and goes to the Dashboard, which is also "the start". | workflows | source; captures |
| F5 | **The assistant is called by its backend's name** in about a dozen places where the user is asking Kaluta (the prompt, the column, the composer, the task dialog, a menu item), against principle 17. | mail window, composer | captures; `AgentStore.swift:101` |
| F6 | **The same reply ends differently:** sent from a workflow the conversation is archived; sent from the mail window it stays, with no *Send and Archive*. | composer | source |
| F8 | **Kaluta's suggestions have no "not yet" and no "none".** Before the background job has run, or with no agent, nothing is shown; Replies shows no suggestion line at all. | Triage, Replies, Waiting | source |
| F14 | **Correcting what Kaluta knows takes three places and has no search.** The page that shows everything edits nothing. | knowledge pages | source; capture |

### Minor (2)

| | Finding | Where |
|---|---|---|
| F7 | Half the Dashboard is below the fold at the default size, including *Done today*, the key hints and the way to Mail. | Dashboard |
| F9 | *What do you want to say?* makes *Send* (as typed) the filled button and *Generate* the secondary one. | workflow writing |
| F10 | Return opens a new window, Space does nothing, and no key moves from the list into the reader. | mail window |
| F11 | Tabs are icons until chosen (Inbox categories, What Kaluta Knows) and have no keys. | mail window |
| F12 | The toolbar is ten or more unlabelled icons and cannot be customised. | mail window |
| F13 | Search results show no query, count or marked matches; exact and *Related* matches run together; no control leaves search. | search |
| F15 | A routine reads as a sentence on its card and as a form when opened; Preview does not say why. | Routines |
| F16 | A tip card, the writing-guide sheet and its banner can stack at first run. | mail window |
| F17 | The mail window says nothing about the day; its only figure is unread. | mail window |
| F18 | Triage offers seven actions, with Reply all and Forward ahead of Archive; Replies arranges the same actions differently. | workflows |
| F19 | Mouse only: category tabs, *Tidy the Inbox*'s Archive, the meeting brief's disclosure. | several |
| F20 | Send is the faintest of five unlabelled icons in the composer. | composer |
| F23 | Esc changes place (mail to Dashboard and back) where the platform uses it to cancel. | both |
| — | E means a different thing in each workflow; waiting cannot be seen from the mail window; three near-identical "tell me later" ideas; a correction's proposal is not shown on its routine. | pages 6, 7, 9 |

### Cosmetic (1)

| | Finding | Where |
|---|---|---|
| F21 | The reading report's third column sits ten points above the others. | Weekly reading |
| F22 | Two voices: sentence case in workflows, title case in the mail window's empty states. | both |

## Against the lenses

1. **Usability heuristics.** Strong on user control (Undo everywhere,
   no confirmations) and on status in workflows. Weak on consistency
   (F4, F5, F6, F18, F22) and on recognition in the mail window (F11,
   F12).
2. **The platform's guidelines.** Menus, standard keys and VoiceOver
   actions are in good order. Against them: an uncustomisable toolbar
   (F12), no prominent primary action in the composer (F20), panes
   without sensible minimums (F1), Esc as navigation (F23).
3. **Keyboard cost.** Workflows: one key a decision. Mail window:
   comparable for archive and label, heavier for reply (a window, then
   archive separately: F6) and for reading (F10).
4. **Glanceability.** The Dashboard's first screen passes the
   five-second test; its second screen and the whole mail window do
   not (F7, F17).
5. **Working with AI.** Good: nothing is learned or sent without
   consent; evidence is a key away; corrections are easy to make
   (*Correct it once*). Missing: what Kaluta can do and how sure it is
   before it has spoken (F8); an efficient way to correct what it
   knows (F14); the routine does not show what a correction changed
   (page 9).
6. **States.** Empty states exist everywhere. Loading and "not yet"
   states for Kaluta's work are the gap (F8).
7. **Accessibility.** Rows and workflows carry VoiceOver actions and
   Reduce Motion is respected in about nineteen places. The mouse-only
   controls (F19) and icon-only tabs (F11) are the gaps. Contrast was
   not measured.

## What changed since October

Of October's findings: getting in costs fewer interruptions (tips one
at a time; F16 is what remains); "nothing says what the job is" is
fixed by the Dashboard and the workflows but not in the mail window
(F17); "a decision spans windows" is fixed in the workflows and open in
the mail window (F6). The crash and the Quit problem were not looked
for.

## Issues filed

Findings rated 3 are filed under epic `oagc-24wb`, each citing its
page. Layout and design-system work is not filed until the maintainer
has chosen.
