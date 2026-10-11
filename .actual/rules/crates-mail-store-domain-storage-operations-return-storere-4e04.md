# Db Persistence Module: Domain Storage Operations Return Storeresult Propagate

These rules are ALWAYS ACTIVE for domain persistence operations and data store modules requiring read or write access to persistent storage.

### Rules

- **R-STORE-001** MUST: Domain storage operations MUST return StoreResult to propagate database failures consistently across module boundaries.

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