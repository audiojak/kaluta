# Db Persistence Module: Persistence Functions Accept Transaction Connection Handles

These rules are ALWAYS ACTIVE for all domain persistence operations, data store modules, and transaction management logic requiring read or write access to persistent storage.

### Rules

- **R-DBP-001** MUST: Persistence functions MUST accept transaction or connection handles passed by the Db closure rather than instantiating independent database handles.

### Verify

```bash
# Find the dependency manifest, identify the build tool, and run tests/verification checks for storage modules
# (e.g., cargo test --package storage-crate)
```

**Accept when:**
- All unit and integration tests executing queries and updates through Db transaction closures pass successfully.
- Persistence methods consistently propagate errors using StoreResult without panicking or leaking underlying database connections.

<enforcement>
Claude Code MUST NOT skip or defer verification. All storage modules must use standard Db closures for transactional safety and uniform error propagation.
</enforcement>