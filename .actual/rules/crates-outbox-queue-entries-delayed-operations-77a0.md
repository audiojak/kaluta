# Exact-delta undo ledger and transactional outbox queue: Outbox Queue Entries Delayed Operations Remain

These rules are ALWAYS ACTIVE for all database migrations, outbox processing loops, and core mutation/compose logic within crates/mail-store, crates/mail-sync, and crates/openagc-core.

### Rules

- **R-OUTBOX-001** MUST: Outbox queue entries for delayed operations MUST remain cancellable via the undo ledger until network dispatch begins.

### Verify

```bash
# Discover and run the project integration tests for mail storage mutations, undo ledger rollbacks, and outbox queue dispatch
cargo test --workspace
```

**Accept when:**
- Message mutations write exact inverted deltas into the undo ledger within the same database transaction.
- Executing an undo operation correctly restores prior message label diffs.
- Delayed outbox queue entries are successfully cancelled and excluded from network dispatch when undone.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>