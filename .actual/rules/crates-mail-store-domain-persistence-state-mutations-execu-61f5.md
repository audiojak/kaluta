# Db Persistence Module: Domain Persistence State Mutations Execute Within

These rules are ALWAYS ACTIVE for all domain persistence operations, data store modules, and transaction management components interacting with persistent storage.

### Rules

- **R-DBP-001** MUST: All domain persistence state mutations MUST execute within transaction closures dispatched through write_blocking on the Db instance.

### Verify

```bash
# Inspect repository configuration to identify the test runner and execute the automated test suite covering storage modules.
# Discover the workspace static analysis tool and run verification checks against data access code to confirm transaction closure compliance.
```

**Accept when:**
- All unit and integration tests executing queries and updates through Db transaction closures pass successfully.
- Persistence methods consistently propagate errors using StoreResult without panicking or leaking underlying database connections.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>