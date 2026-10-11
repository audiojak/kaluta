# Exact-delta undo ledger and transactional outbox queue: Mutations Modifying Message State Record Inverted

These rules are ALWAYS ACTIVE for mutations modifying message state, including database migrations and storage routines in `crates/mail-store`, outbox processing loops in `crates/mail-sync`, and mutation/compose logic in `crates/openagc-core`.

### Rules

- **R-MSG-001** MUST: Mutations modifying message state MUST record inverted deltas in the undo ledger prior to scheduling external sync outbox jobs.

### Verify

```bash
# Discover and run the project integration tests for mail storage mutations, undo ledger rollbacks, and outbox queue dispatch.
cargo test --package mail-store --package mail-sync --package openagc-core
```

**Accept when:**
- Message mutations write exact inverted deltas into the undo ledger within the same database transaction.
- Executing an undo operation correctly restores prior message label diffs.
- Delayed outbox queue entries are successfully cancelled and excluded from network dispatch when undone.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests scheduling outbox jobs without corresponding inverted delta ledger records will be blocked at code review.
</enforcement>