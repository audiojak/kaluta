# Adoption of mail_domain::Millis for Core Temporal Primitives: Public Contracts Persistence Functions Not Expose

These rules are ALWAYS ACTIVE for all code handling data structures, domain models, public contracts, and persistence adapters representing timestamps, execution histories, task due dates, and search filter bounds within storage and query subsystems.

### Rules

- **R-MILLIS-001** MUST_NOT: Public contracts and persistence functions MUST NOT expose raw unvalidated integer values or polymorphic datetime representations where domain timestamps are exchanged across storage boundaries.

### Verify

```bash
# Discover the workspace build and test execution runner and run unit test suites covering query parsing and domain storage operations
cargo test --workspace --bins --tests

# Execute workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration will block pull request merges when unapproved temporal representations are detected.
</enforcement>