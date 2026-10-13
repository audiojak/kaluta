# Adoption of mail_domain::Millis for Core Temporal Primitives: Data Models Representing Scheduled Tasks Execution

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-MAIL-001** SHOULD: Data models representing scheduled tasks, execution runs, and routine state SHOULD derive standard structural serialization, equality, and debugging traits to ensure uniform interoperability across transactional boundaries.

### Verify

```bash
# Discover the workspace build and test execution runner to run all unit test suites covering query parsing and domain storage operations.
# Execute the workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints.
# Run database round-trip test suites to confirm millisecond timestamp persistence and retrieval fidelity across routines and tasks.
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests are verified by automated continuous integration running static analysis, unit tests, and architecture review.
</enforcement>