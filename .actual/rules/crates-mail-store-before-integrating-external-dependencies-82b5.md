# Db Persistence Module: Before Integrating External Dependencies Supporting Persistence

These rules are ALWAYS ACTIVE for all domain persistence operations, data store modules, and integration of external dependencies supporting persistence or serialization.

### Rules

- **R-DBP-001** MUST: Before integrating external dependencies supporting persistence or serialization, the consumer MUST inspect the repository lock artifact to verify the exact resolved dependency versions.
- **R-DBP-002** MANDATORY: Before writing code that uses a versioned library, execute the lock-version grounding process in order (find dependency manifest, identify build tool, inspect repository lock or resolution artifact, look up official documentation for that exact version, confirm every API/class/function exists in that version's documentation, and re-run steps 3-5 per dependency at point of use).

### Verify

```bash
# Inspect repository configuration to identify the test runner and execute the automated test suite covering storage modules
# Discover the workspace static analysis tool and run verification checks against data access code to confirm transaction closure compliance
```

**Accept when:**
- All unit and integration tests executing queries and updates through Db transaction closures pass successfully.
- Persistence methods consistently propagate errors using StoreResult without panicking or leaking underlying database connections.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>