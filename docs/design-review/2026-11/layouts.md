# Mail window layouts

Part 2 of the review (`docs/plans/ux-review-2026-11.md`). Five
arrangements of the mail window, each drawn with the demo mailbox's
content. Open the pages in a browser; each has buttons for light and
dark, a 1200 and an 820 point window, and (C and D) the second state.

| Option | Page |
|---|---|
| A. Three columns, refined | [layout-a-three-columns.html](mockups/layout-a-three-columns.html) |
| B. Two panes | [layout-b-two-panes.html](mockups/layout-b-two-panes.html) |
| C. One column | [layout-c-one-column.html](mockups/layout-c-one-column.html) |
| D. List with a peek | [layout-d-peek.html](mockups/layout-d-peek.html) |
| E. Grouped by what is needed | [layout-e-grouped.html](mockups/layout-e-grouped.html) |

They are drawings to choose from, in a neutral style so that the
arrangement is judged apart from the design directions
([design-system.md](design-system.md)). `mockups/build.py` writes them.

## What is wrong with today's window

Measured on the captures in `img/` (1200×760, the size of the October
baseline):

- **It is four columns, not three.** Sidebar, list, reader and Kaluta's
  column. With Kaluta's column open the reader is about 250 points wide
  (`img/main-agent-light.png`): four or five words a line, and a short
  message runs to fifteen lines.
- **At 820 points the reader is about 200 points** with Kaluta's column
  closed (`img/narrow-light.png`): two or three words a line. The list
  keeps its full width and the reader takes all of the loss.
- **The list shows six conversations** in 760 points once the category
  tabs and a tip card are above it.
- **Three regions scroll**, and a fourth when Kaluta answers.
- **It says nothing about the day.** "Primary · 94 unread" is the only
  figure; what is new and what needs the user is on the Dashboard, a
  different place.

The reader is where mail is read and it is the column that gives way
first. Any option has to fix that before anything else.

## How each was judged

Against the ten jobs of the plan, on four things:

- **Keys:** the keys and screen changes to do the job.
- **Glance:** what the window says about the day without acting.
- **Reading room:** the reader's width at 1200 and at 820 points.
- **Fit:** how it sits with the workflows, the platform and what exists.

Scores are 1 (poor) to 5 (good), mine from the drawings and the code,
not from use.

| | A. Three columns | B. Two panes | C. One column | D. Peek | E. Grouped |
|---|---|---|---|---|---|
| Conversations in view at 760 high | 7 | 8 | 10 | 17 | 8, in four groups |
| Reader width at 1200 | 650 | 800 | 1200 (680 column) | 745 | 800 |
| Reader width at 820 | 465 | 500 | 820 (680 column) | 640 | 490 |
| Open, then next conversation | ↓ | ↓ | Return, J | Space, J | ↓ |
| Go to another mailbox | click, or ⌘K | ⌘K, or one click then one | ⌘K | ⌘K | ⌘K |
| Keys | 4 | 4 | 4 | 4 | 5 |
| Glance | 3 | 3 | 3 | 4 | 5 |
| Reading room | 3 | 4 | 5 | 4 | 4 |
| Fit | 5 | 4 | 3 | 3 | 4 |
| **Total** | **15** | **15** | **15** | **15** | **18** |

The four pane arrangements come out level, each strong where another is
weak. The grouping (E) is a different kind of change, and it is the one
that moves the score.

## What each is for

- **A** is the least change. Its two fixes (Kaluta's column becomes a
  line and an overlay; the sidebar gives way before the reader) are
  worth making whatever else is chosen.
- **B** is A without the sidebar. It gains 150 points of reading room
  and loses the counts for other mailboxes. It goes against the
  platform's guidance not to hide a sidebar by default, unless the
  sidebar stays one key away and comes back at wide sizes.
- **C** is the workflows' way of working applied to all mail. It is the
  calmest and reads best, and it costs the most to build and to learn:
  a click on a row stops meaning "select".
- **D** is for scanning and tidying many conversations. It loses the
  two-line preview, which is what lets the user decide without opening.
- **E** answers "what needs me?" in the mail window itself. It depends
  on Kaluta's sorting being right and easy to correct.

## Recommendation

**The grouped list (E) in two panes (B), with the sidebar one key
away, built in three steps.**

1. **Give the reader its room (from A).** Kaluta's column becomes the
   one-line prompt under the reader that is already there, and its
   answers open over the reader, not beside it. The reader never goes
   below about 480 points: the sidebar narrows to a strip first, then
   hides. This is the fix for the worst layout finding and changes
   nothing else.
2. **Make the sidebar optional (B).** A switcher at the top of the list
   (and ⌘K, which is there) reaches every mailbox, label and page, with
   counts. The sidebar is shown at wide sizes and hidden below about
   1,000 points, with ⌃⌘S and a toolbar button either way, which keeps
   to the platform's guidance.
3. **Offer the grouped view as "Today" (E).** Answer, Decide, Waiting
   on others, Read when you can, from the triage suggestions that exist
   already (no new inference). The plain Inbox by date stays one key
   away and is what shows when no agent is ready. Moving a conversation
   to another group is one key and counts as a correction (*Correct it
   once* already does this for routines).

Step 3 is where the mail window and the Dashboard start to merge: the
Dashboard's cards and the groups are the same sets. Whether the
Dashboard stays a separate place is a question for the maintainer once
the grouped view exists; the review does not settle it.

**Not recommended now:** C and D. C is worth returning to if the
grouped view succeeds, since each group is then a short queue worked one
at a time, which is what C draws. D's one-line rows suit Clean Up, which
has its own surface.

## Open questions for the maintainer

- Should the grouped view be the default, or a view the user turns on?
- Does Kaluta's column have uses the overlay cannot serve (a long
  conversation with the agent beside a message)?
- Are label colours from the mail service to stay as they are in the
  list? (They compete with any colour the groups carry.)
