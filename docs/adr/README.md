# Architecture decision records

Decisions that change OpenAGC's architecture, one per file, numbered in
order (`NNNN-short-title.md`). Each says which entry of the register in
[ADR 0001](0001-architecture-decision-register.md) it amends, and the spec
is updated in the same change so the two agree. Smaller product decisions
stay in the spec's dated amendments and the nightly plans in `docs/plans/`.

| ADR | Decision | Date |
| --- | --- | --- |
| [0001](0001-architecture-decision-register.md) | Architecture decision register (baseline) | 2026-09-23 |
| [0002](0002-imap-first-sync.md) | IMAP-first sync for Gmail accounts | 2026-09-28 |
| [0003](0003-tiered-download.md) | Tiered download: headers for the window, bodies where they matter | 2026-09-27 |
| [0004](0004-one-store-per-account.md) | Multiple accounts: one store per account, one visible at a time | 2026-09-26 |
| [0005](0005-archive-accounts.md) | Imported mailboxes are accounts without a provider | 2026-09-26 |
| [0006](0006-exact-undo.md) | Act, then offer an exact undo | 2026-09-27 |
| [0007](0007-read-only-embedded-agent-sessions.md) | Embedded AI runs in read-only agent sessions | 2026-09-29 |
| [0008](0008-tasks-local-with-gmail-label.md) | Tasks live on the Mac; Gmail sees them as a `Task` label | 2026-09-29 |
| [0009](0009-test-and-scratch-isolation.md) | Tests and scratch runs are isolated inside the app's bundle id | 2026-09-28 |
| [0010](0010-design-system-in-code.md) | A design system in code, enforced by lint | 2026-09-28 |
| [0011](0011-writing-guide.md) | A per-account writing guide, learned from sent mail through the user's own agent | 2026-09-29 |
| [0012](0012-global-facts-store.md) | One global store for facts the user makes global | 2026-10-05 |
| [0013](0013-recording-ai-compositions.md) | Every AI composition is recorded, then compared with what was sent | 2026-10-05 |
| [0014](0014-agent-mailboxes.md) | Agent mailboxes are accounts on an agent-mail service, created in the app | 2026-10-06 |
| [0015](0015-service-accounts.md) | Agent mailboxes belong to service accounts | 2026-10-08 |
| [0016](0016-rules-server.md) | A rules server serves agent mailboxes' guides and facts to cloud agents | 2026-10-08 |
