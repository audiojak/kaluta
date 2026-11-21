# UX review: the mail window and the Dashboard — 2026-11-19

An overnight review of the two places the user spends the day: the mail
window (sidebar, conversation list, reader, agent column, composer) and
the Dashboard with its workflows (Triage, Replies, Tasks, Waiting,
Reading, Teach Kaluta, the knowledge and routine pages). It follows the
October baseline (`docs/design-review/`, plan `docs/plans/design-review.md`)
and adds two things that one did not do: a direction for a design system
that is Kaluta's own, and other layouts for the mail window, mocked up.

Nothing in the app changes overnight. The output is findings, mock-ups
and a proposal; what to build is decided from them afterwards.

## Decisions (maintainer, 2026-11-19)

1. **Scope:** the mail window and the Dashboard. Onboarding, Settings
   and the menu-bar surfaces are looked at only where a daily job
   passes through them.
2. **Reference products are compared from their public documentation**
   (help pages, product pages, published guidelines), not run side by
   side.
3. **No product names in the repository.** References are named by the
   pattern they stand for (below). The list of products is kept outside
   the repository. This holds for the report, mock-ups, issues and
   commit messages.
4. **Kaluta should feel like itself.** The review proposes how: a new
   design system, or changes that make the present one distinct.
5. **The three-column mail window is open to question.** The review
   considers other patterns and mocks them up.

## What is examined against

### House rules

- The 17 workflow principles in `docs/plans/design-review.md`.
- The four design-system principles in `docs/design-system.md`. The
  first, *the system first*, pulls against decision 4; part 3 below
  settles how far each goes.

### Outside lenses

1. **The ten usability heuristics**, each finding rated for severity:
   0 not a problem, 1 cosmetic, 2 minor, 3 major, 4 blocks the job.
2. **The platform's interface guidelines for macOS:** menus, toolbar,
   focus, standard keys, window behaviour.
3. **Keyboard cost per job:** keys pressed, screen or mode changes, and
   every place the mouse is required.
4. **Glanceability** (the Dashboard above all): within five seconds,
   what needs the user and what to do first.
5. **Guidelines for people working with AI** (two published sets, one
   as a cross-check): what Kaluta did, why, how sure, how to correct
   it, and what it learns from the correction.
6. **States:** empty, loading, error, offline and "nothing left" on
   every surface.
7. **Accessibility:** VoiceOver labels and order, contrast, Reduce
   Motion, larger text, full keyboard access.

### Reference patterns

Each area is compared with two to four products known for it, described
here by what they do well.

| Area | Patterns to compare with |
|---|---|
| Email | the platform's own mail app; a native client for the same mail service; a keyboard-first client built for speed |
| Triage | single-key decisions over a split inbox; a screener that asks once per sender; an issue tracker's one-at-a-time triage queue |
| Replies | a reply-later stack worked through in one sitting; instant reply with stored snippets; AI drafts the user edits before sending |
| Tasks | a calm Today list with quick entry; a keyboard-driven tracker; dates typed in plain words |
| Knowledge | pages that show and let the user edit what an assistant remembers; filtered database views; per-person contact cards |
| Rules | rules read as sentences with a preview of what they match; automation built from readable steps; sorting that learns from corrections; the mail service's own filters as the baseline to beat |

## The jobs walked

Each is walked start to finish in the demo, in the mail window and,
where one exists, in its workflow, so the two can be compared.

1. Open the app in the morning and find out what needs attention.
2. Triage what is new.
3. Answer what needs a reply.
4. Find one conversation from last month and act on it.
5. Write a new message from nothing, with and without Kaluta.
6. Work today's tasks; make a task from an email.
7. Check what is being waited on and chase one.
8. See what Kaluta knows about a person and correct one fact.
9. Read a sorting routine, change it, and undo a wrong filing.
10. Leave mid-job and come back.

## Parts

### Part 1: the review

For each job: the steps as keys and clicks; dead ends and mode changes;
waits; how each lens and house rule is met or missed; the nearest
reference pattern and the gap to it; screenshots, light and dark.
Findings carry a severity and the surface they are on.

### Part 2: mail window layouts

The present window is sidebar, list, reader, with the agent column
beside them. At least four alternatives, each mocked up with the demo
mailbox's content, light and dark, at 1200×760 and at a narrow width:

- **A. Three columns, refined:** the present layout with its worst
  findings fixed. The control.
- **B. Two panes:** list and reader; mailboxes and labels move to a
  switcher and the command palette.
- **C. One column:** the list fills the window and a conversation opens
  in its place, as the workflows show one thing at a time.
- **D. List with a peek:** the list keeps the window and a conversation
  slides over part of it, closing with Esc.
- **E. Grouped by what is needed:** the Inbox as sections (answer,
  decide, read, waiting) rather than one list by date.

Where Kaluta's agent sits in each is part of the option. Each option is
scored against the jobs above (keyboard cost, glanceability, room for
the reader) with what it gains and loses, and one is recommended.

### Part 3: a design system that is Kaluta's own

- An audit of what makes the app look like any system app today, and
  what is already its own (the workflow colours, keycaps, the pill).
- Where distinctness is safe and where it costs: the platform's
  behaviour (menus, keys, focus, accessibility) stays; type, colour,
  shape, density, motion, illustration, sound and the words Kaluta
  uses are open.
- Three directions, each a one-page style tile (type, colour, shape,
  motion, voice) and the same two screens redrawn in it: the mail
  window and the Dashboard.
- A recommendation, the changes to `docs/design-system.md` it implies
  (tokens and principles, including what becomes of *the system
  first*), and the order to adopt them in.

## Method

**Running it.** `scripts/test-macos.sh build`, then `scripts/demo.sh`:
the workflow demo in a throwaway data directory, with fake agents.
Screens are captured with `scripts/snapshot.sh` and computer use.

**Safety.** The Debug build on the demo account with
`-KalutaFakeAgents YES` only. Never the real account, never real sync,
never the installed app. Nothing under
`~/Library/Application Support/Kaluta` or `…/OpenAGC` is touched.

**Mock-ups** are static HTML pages, one per option, self-contained,
light and dark, using the demo mailbox's content. They are drawings to
choose from, not prototypes of behaviour.

**Limits known beforehand.** Fake agents answer at once, so waits on a
model are taken from the code. Some pure-SwiftUI content captures
blank; a page says *unconfirmed* where that may be so. Reference
patterns come from public documentation, which may trail the products.

**Branch.** `overnight-2026-11-19`, pushed after each part. Nothing is
pushed to `main`.

## Output

`docs/design-review/2026-11/`:

- `README.md`: the short version, findings across jobs ranked by
  severity, and the three recommendations (fixes, layout, design
  system).
- one page per job;
- `layouts.md` and `mockups/layout-*.html`;
- `design-system.md` and `mockups/direction-*.html`;
- `img/`: screenshots.

Findings of severity 3 or 4 are filed as beads issues under one epic,
each citing its page. Layout and design-system work is filed only once
the maintainer has chosen.
