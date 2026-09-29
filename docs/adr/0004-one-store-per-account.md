# ADR 0004: Multiple accounts: one store per account, one visible at a time

- Status: Accepted
- Date: 2026-09-26
- Amends: spec §1.2 (multi-account was an MVP non-goal); ADR 0001
  register, **Storage** and **Core**
- Spec: §7.7

## Context

The maintainer asked for several Gmail accounts, switched from an avatar
button. The store, sync service, routines and agent sessions were all
written for one account.

## Decision

- An account is a directory `accounts/<id>/` with its **own**
  `mail.sqlite`, attachments cache, routines, agent sessions and Keychain
  items; `accounts/index.json` lists them.
- **Nothing is merged across accounts**: no unified inbox, no cross-account
  search, no cross-account agent tools. An agent session is bound to the
  account it started on and never sees another account's mail.
- The core holds every opened account and syncs all of them in the
  background (notifications and local routines keep running); only the UI
  shows one at a time. Every `CoreEvent` carries its account id, and the
  window ignores other accounts' events except new-mail notifications and
  unread counts.
- Per-account state lives in the store (undo stack, tasks, categories)
  or in defaults keyed by account id.

## Consequences

- Isolation by construction: a query cannot join across accounts because
  the data is in different files. The agent permission model needs no
  cross-account checks.
- Removing an account is deleting a directory (after the Keychain items).
- Features that would want a cross-account view (a unified inbox, one
  task list for all accounts) need an explicit design; they cannot fall
  out of a query.
- More open SQLite files and sync services; acceptable at the handful of
  accounts expected.

## Alternatives considered

- **One database with an `account_id` column**: every query and index
  gains a filter, and a missed filter leaks mail between accounts.
