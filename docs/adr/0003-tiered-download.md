# ADR 0003: Tiered download: headers for the window, bodies where they matter

- Status: Accepted
- Date: 2026-09-27
- Amends: ADR 0001 register, **Storage** (what a synced message holds)
- Spec: §7.4 (amendment 2026-09-27, measured 2026-09-28)

## Context

Most old mail is never opened. Over IMAP (ADR 0002) headers are cheap and
bodies are not; downloading every body in a six-month or "everything"
window costs bandwidth, time and disk for mail nobody reads.

## Decision

An account on IMAP downloads in two tiers:

- **Headers** (plus the first ~2 KB of text, for snippets) for the whole
  sync window. Lists, labels, sorting and routines' first pass work from
  these.
- **Bodies** for the Inbox and a body window (default the last 30 days;
  Settings › Accounts › *Full messages for*), and on demand: a message the
  user opens, one an agent or routine reads (`ensure_bodies` before the tool
  answers), and search matches.

Each age tier has its own queue priority; headers-only is that priority
+ 10, which the body backfill never drains. Accounts on the REST API keep
bodies for the whole window, since headers cost the same quota there.

## Consequences

- Measured on a 100,000-message fake mailbox: Inbox browsable in 1.8 s,
  every message listed in 7.4 s, no body outside the window fetched.
- Text search over old mail relies on Gmail's server search; matches then
  download like opened messages.
- An agent reading old mail pauses while its bodies download.
- A `body_state` of `metadata` is a normal, long-lived state, not a
  transient one; every reader of bodies must handle it.

## Alternatives considered

- **Every body in the window**: the REST behaviour; too slow and large for
  "everything".
- **Bodies only on open**: no offline reading or local full-text search for
  recent mail.
