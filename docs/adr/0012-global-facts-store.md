# ADR 0012: One global store for facts the user makes global

- Status: Accepted
- Date: 2026-10-05
- Amends: ADR 0004 ("nothing is merged across accounts"), for this one
  explicit, user-chosen case; ADR 0001 register, **Storage**
- Builds on: ADR 0006 (exact undo), ADR 0011 (writing guide)
- Spec: §14.11; plan `docs/plans/analysis.md`

## Context

Facts about the user (their role, calendar link, time zone, the people
they mention) used to be writing-guide entries in category F3, one set
per account. Most of them are the same on every account the user has,
and the maintainer asked that a fact can be made global in one step and
that global facts are listed in Settings.

ADR 0004 keeps every account's data in its own store and merges nothing
across accounts. Copying a fact into each account would make "global"
mean "copied", and an edit on one account would not reach the others.

## Decision

- **One global store**, `data_dir/global/facts.sqlite`, with the same
  `facts`, `fact_evidence` and `fact_categories` schema as the account
  stores. The core opens it once, lazily, and every account reads it.
- **Only facts and their custom categories live there.** Nothing else
  crosses accounts: no mail, no guide entries, no evidence from another
  account's mail. A global fact carries no evidence quotes, which would
  show one account's mail on the others: the quotes stay in the account's
  change record, and undo brings them back with the fact.
- **The user chooses.** Facts are per account by default. *Make Global*
  moves a fact into the global store; *Make This Account's Only* moves it
  back to the account the user is on. A custom category goes with the
  fact as a copy (other facts may still use it where it was), and only
  when the other side has none by that key. Moving refuses when the other
  side already has a fact with that category and label. Nothing becomes
  global by inference or by an agent.
- **Overrides:** an account fact with the same category key and label as a
  global fact wins for that account. Rendering merges global facts under
  account ones.
- **Undo across two stores** is one change on the account's undo stack
  (ADR 0006): the change record in the account's store snapshots both
  sides (the account row and the global row, before and after), and undo
  writes both back. Edits made directly in Settings › Facts are recorded
  in the global store and undone from the open account's undo stack (with
  no account open they are made without undo).
- **Removing an account** never removes global facts.

## Consequences

- The first store outside `accounts/`; backups, export and "remove all
  data" have to know about it.
- A global edit shows on every account at once, which is what the user
  asked for, and an agent on one account can read facts the user entered
  on another. Only facts the user made global do this.
- Two writers (an account store and the global store) take part in one
  change. The core applies the global side first and the account side in
  the same account transaction that records the change, so a failure
  leaves at worst a duplicate the next move reconciles, never a lost fact.

## Alternatives considered

- **Copy the fact into every account**: edits and deletions drift, and a
  new account starts without them.
- **A `global` flag inside one account's store**: makes one account the
  owner, and removing it loses everyone's facts.
- **Facts in user defaults**: no evidence, no undo, no versions, and
  defaults are not meant for user data.
