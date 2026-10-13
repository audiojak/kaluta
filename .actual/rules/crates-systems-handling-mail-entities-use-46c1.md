# Adoption of mail_domain Core Domain Identity Types: Systems Handling Mail Entities Use Strongly

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-MAIL-001** MUST: Systems handling mail entities MUST use the strongly typed identifier types from the mail_domain module, specifically mail_domain::MessageId, for all cross-module contracts, queue operations, and message retrieval interfaces.

### Verify

```bash
# Discover the workspace build harness and execute the complete test suite to confirm type checking and interface compatibility.
cargo test --workspace
# Discover the repository static analysis and linting configuration to verify public contracts adhere to domain identifier typing.
cargo clippy --workspace --all-targets
```

**Accept when:**
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>