# Adoption of mail_domain::Millis for Core Temporal Primitives: Query Parsing Interfaces Convert Human Readable

These rules are ALWAYS ACTIVE for data structures, domain models, storage subsystems, and query parsing interfaces representing timestamps, execution histories, task due dates, and search filter bounds.

### Rules

- **R-MAIL-001** MUST: Query parsing interfaces MUST convert human-readable date filters into mail_domain::Millis epoch values before emitting parsed abstract syntax tree structures.

### Verify

```bash
# Discover workspace build and test execution runner to run all unit test suites covering query parsing and domain storage operations.
# Execute workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints.
# Run database round-trip test suites to confirm millisecond timestamp persistence and retrieval fidelity across routines and tasks.
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is enforced via automated continuous integration pipeline running static analysis and unit test suites on every pull request, and architecture peer review blocking non-conforming changes.
</enforcement>