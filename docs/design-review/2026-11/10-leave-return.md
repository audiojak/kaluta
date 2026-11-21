# 10. Leave mid-job and come back

**Where:** between the Dashboard, a workflow and the mail window.

## How it goes today

Esc in a workflow leaves it; the set is kept for the day and resumes
where it was. Esc or M on the Dashboard opens the mail window. Esc in
the mail window (with workflows on) goes to the Dashboard. ⌘K lists
"Start Triage" and the rest among its commands.

## Findings

- **F4 (3). One place has three names, and the button to it has a
  fourth.** Every workflow's back button reads "‹ Mail Esc" and goes to
  the Dashboard (`leaveWorkflow` only clears the session). The end
  screen calls the same place "the start" ("Back to the start"); the
  sidebar and the View menu call it "Dashboard". Heuristics 2 and 4.
- **F2 (3). Starting a workflow from ⌘K in the mail window shows
  nothing.** The palette lists "Start Triage" there; `beginWorkflow`
  creates the session but the window shows a workflow only when it is
  on the Dashboard side (`showsWorkflows` needs `home == .start`), and
  nothing switches it. From the source, not run.
- **F3 (3). ⌘0 is bound twice:** *Window › Mail* and *View › Dashboard*
  (`KalutaApp.swift:204`, `:212`). Which one wins was not run.
- **F23 (2). Esc navigates.** On the platform Esc cancels. Here it
  moves between the app's two halves, in both directions, so an Esc
  meant to dismiss something that has already gone leaves the mail.
  M already does the same job from the Dashboard.
- **(2) Nothing in the mail window says a set is half done.** The
  Dashboard's card says "in progress"; the sidebar's Dashboard entry
  does not.

## What is good

Resuming is exact: same set, same place, decisions kept, and anything
handled elsewhere meanwhile is marked done rather than shown again.
Leaving never asks a question.

## Against the reference patterns

An issue tracker moves between its places with two-key chords that
name the place (go, then a letter); Kaluta's ⌘1–6 and ⌘K cover this.
None of the references uses Esc to change place.
