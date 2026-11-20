# 1. Open the app and find out what needs attention

**Where:** the Dashboard (`StartScreenView`), or the mail window when
workflows are off (the default for a new account).

## How it goes today

With workflows on, launch shows the Dashboard: the date, a headline
("About 20 minutes of mail today."), the next meeting if a brief is
ready, the catch-up line ("18 new since 9:00 · 3 need you"), the next
workflow as a large card with Return on it, six or seven smaller cards
on the keys 1–8, then *Tidy the Inbox*, *Done today* and a footer.
Return starts the next workflow: one key from launch to the first
decision.

With workflows off, launch shows the mail window: "Primary · 94 unread"
and a list by date.

## Findings

- **F7 (2). Half the Dashboard is below the fold.** Its content is
  1,344 points tall in the 760-point default window (view tree,
  2026-11-20). The headline, the next card and the six cards fit;
  *Tidy the Inbox*, *Done today*, the key hints ("Return starts · 1–7
  chooses") and *Open Mail* do not. The reward for work done and the
  way out are the two things a scroll away. Lens 4 (glanceability).
- **F17 (2). The mail window says nothing about the day.** Its only
  figure is the category's unread count. What is new and what needs
  the user is known (the catch-up line) but shown only on the
  Dashboard. A user who works in the mail window never sees it.
  Principle 12.
- **F16 (2). Getting in can still cost several interruptions.** Tip
  cards now come one at a time (Daily Workflows, Clean Up, Categories,
  Important Only, the agent), which fixes most of October's finding 3.
  But the writing-guide invitation sheet can open over a tip card, and
  its banner can then sit above one (`MainWindow.swift:298`).
- **F19 (2). Two Dashboard controls need the mouse:** the meeting
  brief's disclosure and each *Archive* in *Tidy the Inbox*.
- **F22 (1). Two voices.** The Dashboard and workflows write in
  sentence case with a full stop ("Nothing waiting here."); the mail
  window's empty states use title case ("Nothing Here Yet", "Choose One
  to Open It").

## What is good

The headline is a real estimate from the user's own pace, the next
step is one key, and counts are honest (a card with nothing in it is
dimmed and says so). This is the strongest screen in the app and the
most its own.

## Against the reference patterns

A split inbox shows a count per section that disappears at zero; the
Dashboard's cards do the same and add time. A calm Today list puts the
day's calendar above the tasks; the next-meeting line is the start of
that. Neither reference makes the user leave their mail to see it: the
gap is that Kaluta's summary and Kaluta's mail are two places
([layouts.md](layouts.md), option E).

*The Dashboard could not be pictured (it captures blank without screen
recording); this page is from its view tree and source.*
