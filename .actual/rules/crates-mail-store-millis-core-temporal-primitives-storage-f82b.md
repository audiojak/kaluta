# Adoption of mail_domain::Millis for Core Temporal Primitives: Storage Task Management Search Parsing Routine

These rules are ALWAYS ACTIVE for all storage, task management, search parsing, and routine execution components handling point-in-time timestamps and duration boundaries.

### Rules

- **R-TEMP-001** MUST: Storage, task management, search parsing, and routine execution components MUST use mail_domain::Millis as the canonical data type for all point-in-time timestamps and duration boundaries across domain models and persistence boundaries.

### Verify

```bash
# Discover workspace build and test execution runner, then execute unit test suites covering query parsing and domain storage operations
# Execute workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints
# Run database round-trip test suites to confirm millisecond timestamp persistence and retrieval fidelity across routines and tasks
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory on every pull request and code change involving temporal primitives.
</enforcement>