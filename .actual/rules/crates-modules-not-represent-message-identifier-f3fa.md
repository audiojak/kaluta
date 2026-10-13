# Adoption of mail_domain Core Domain Identity Types: Modules Not Represent Message Identifiers Raw

These rules are ALWAYS ACTIVE for all workspace crates and modules defining interfaces for message queuing, persistent storage retrieval, message drafting, analytical sampling, and internal service boundaries passing email entity references between storage and orchestration components.

### Rules

- **R-MAIL-001** MUST_NOT: Modules MUST NOT represent message identifiers as raw strings or generic scalar types across public crate interfaces or persistence query boundaries.

### Verify

```bash
# Discover the workspace build harness and execute the complete test suite to confirm type checking and interface compatibility.
# Discover the repository static analysis and linting configuration to verify that public contracts adhere to domain identifier typing standards.
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

**Accept when:**
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration blocks merge on compile errors or lint violations where raw strings are substituted for domain types.
</enforcement>