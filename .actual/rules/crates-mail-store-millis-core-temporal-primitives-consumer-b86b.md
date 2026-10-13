# Adoption of mail_domain::Millis for Core Temporal Primitives: Consumers Inspect Workspace Dependency Manifest Package

These rules are ALWAYS ACTIVE for data structures, domain models, storage subsystems, query parsing, background task management, and public contracts handling timestamps and temporal primitives.

### Rules

- **R-MAIL-001** MUST: Consumers MUST inspect the workspace dependency manifest and package resolution lock artifact to establish the exact locked version of internal domain dependencies prior to implementation.

### Verify

```bash
# Discover workspace build and test execution runner, then execute unit tests covering query parsing and domain storage operations
cargo test --workspace --bins --tests

# Execute workspace static analysis and linting suites
cargo clippy --workspace --all-targets -- -D warnings

# Run database round-trip test suites for millisecond timestamp persistence and retrieval fidelity
cargo test --test persistence_roundtrip
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive (`mail_domain::Millis`).
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration pipeline running static analysis and unit test suites on every pull request will fail if unapproved temporal representations are detected.
</enforcement>