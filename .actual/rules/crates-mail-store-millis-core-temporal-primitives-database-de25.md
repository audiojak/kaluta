# Adoption of mail_domain::Millis for Core Temporal Primitives: Database Storage Adapters Persist Mail Domain

These rules are ALWAYS ACTIVE for storage adapters, query parsing, background routines, and task tracking records within the project.

### Rules

- **R-MAIL-STORE-001** MUST: Database storage adapters MUST persist mail_domain::Millis representations as 64-bit integer values to preserve numerical ordering and index compatibility.

### Verify

```bash
# Discover workspace build and test execution runner to run unit test suites covering query parsing and domain storage operations
# Execute workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints
# Run database round-trip test suites to confirm millisecond timestamp persistence and retrieval fidelity across routines and tasks
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>