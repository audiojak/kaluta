# Db Persistence Module: Transient Scratch Instances Used During Verification

These rules are ALWAYS ACTIVE for domain persistence operations, data store modules, transaction management, and state transitions requiring read or write access to persistent storage.

### Rules

- **R-DB-001** SHOULD: Transient scratch instances of Db used during verification SHOULD clean up associated storage resources upon teardown.

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