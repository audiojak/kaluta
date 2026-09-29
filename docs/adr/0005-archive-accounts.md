# ADR 0005: Imported mailboxes are accounts without a provider

- Status: Accepted
- Date: 2026-09-26
- Builds on: ADR 0004
- Spec: §7.8

## Context

Users have old mail as mbox files (Google Takeout gives one per account).
They want to read, search, sort and ask the agent about it, without
uploading it anywhere.

## Decision

An mbox import creates an **archive account**: an account in every sense
of ADR 0004 (own directory, store, routines, agent sessions, entry behind
the avatar button) with `kind: "archive"` and **no provider**: no Keychain
items, no sync service, no outbox. The importer writes through the same
`MailWriter` that sync uses.

- Message ids are content hashes (first 16 bytes of SHA-256), so importing
  the same file twice is idempotent; `Message-ID` skips duplicates across
  files. Threads come from `X-GM-THRID` when present, else `References` /
  `In-Reply-To` (no subject-only threading).
- Takeout's `X-Gmail-Labels` become labels. Label changes are local and
  allowed: sorting an archive is a main use.
- It cannot send, reply, forward, sync or draft; the UI does not offer
  those commands and the core refuses them.

## Consequences

- Search, the agent, routines and tasks work on archives with no special
  cases, because they only see a store.
- Every provider-backed feature must tolerate an account with no provider
  (`sync_service()` is `None`), as the demo mailbox already does.
- An archive can drift from the Gmail account it came from; they are not
  linked.

## Alternatives considered

- **Import into the Gmail account's store**: mixes mail that exists on the
  server with mail that does not, and sync would fight it.
- **Upload to Gmail**: against the "nothing is uploaded" promise, and slow.
