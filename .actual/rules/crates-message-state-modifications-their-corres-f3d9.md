# Exact-delta undo ledger and transactional outbox queue: Message State Modifications Their Corresponding Inverted

These rules are ALWAYS ACTIVE for crates/mail-store database migrations and storage routines, crates/mail-sync outbox processing loops, and crates/openagc-core mutation and compose logic.

### Rules

- **R-UNDO-001** MUST: Message state modifications and their corresponding inverted undo ledger records MUST be committed within the same database transaction.

### Verify

```bash
# Discover and run the project integration tests for mail storage mutations, undo ledger rollbacks, and outbox queue dispatch.
cargo test --test mail_store_mutations --test undo_ledger --test outbox_dispatch
```

**Accept when:**
- Message mutations write exact inverted deltas into the undo ledger within the same database transaction.
- Executing an undo operation correctly restores prior message label diffs.
- Delayed outbox queue entries are successfully cancelled and excluded from network dispatch when undone.

<enforcement>
Claude Code MUST NOT skip or defer verification. Violation handling: Pull requests scheduling outbox jobs without corresponding inverted delta ledger records will be blocked at code review.
</enforcement>