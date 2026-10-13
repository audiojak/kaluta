# Adoption of mail_domain Core Domain Identity Types: Consumers Verify Dependency Specifications Against Repository

These rules are ALWAYS ACTIVE for all workspace crates and modules defining interfaces for message queuing, persistent storage retrieval, message drafting, and analytical sampling, as well as internal service boundaries passing email entity references between storage and orchestration components.

### Rules

- **R-MAIL-001** MUST: Consumers MUST verify dependency specifications against the repository lock or resolution artifact to ensure exact version consistency before integrating or updating mail_domain types.

### Verify

```bash
# Discover the workspace build harness and execute the complete test suite
# (e.g., cargo test --workspace)

# Discover the repository static analysis and linting configuration to verify domain identifier typing standards
# (e.g., cargo clippy --workspace)
```

**Accept when:**
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration automated build and test pipeline executions, compiler type checking, and workspace static analysis rules enforce these requirements.
</enforcement>