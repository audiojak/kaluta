# ADR 0025: ⌘K is the command palette; commands have one definition

- Status: Accepted
- Date: 2026-11-18
- Amends: spec §14.3 (thread list keys), §14.7 (asking the agent)

## Context

Every action in the mail window has a menu item, and many a single key
in the thread list, but nothing lists them by name. The keys were a
`switch` in the list's `keyDown`, separate from the menus, so a command
could be named or guarded differently in each place. ⌘K put the cursor
in the agent's prompt.

## Decision

1. `MailCommand` (id, title, keys, the list key, why it cannot run now,
   what it does) is the definition of a mail-window command.
   `AppModel.mailCommands` lists them: actions on the selected mail,
   starting a workflow, and going to each mailbox and label.
2. ⌘K opens the command palette: a field over that list. Names that
   start with what is typed come first, then names with a word that
   does, then names that contain it. A command that cannot run now is
   listed greyed out with the reason and Return skips it.
3. The last row is always "Ask Kaluta: <text>", so ⌘K, a question and
   Return still asks the agent. ⌥⌘K puts the cursor in the agent's
   prompt, as ⌘K did (⇧⌘K is the composer's link).
4. The thread list's single keys run the same `MailCommand`s. The
   pickers anchored at a row (`l` Label, `b` Remind) and `j`/`k` stay
   with the list, which owns their place on screen.

## Not done here

The menu bar's items still call the model directly; they are not built
from the list. They call the same functions the commands do, so
behaviour cannot differ, but a new command must be added in both places.
The shortcut guide is likewise still written by hand.
