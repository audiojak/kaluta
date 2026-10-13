# Adoption of mail_domain Core Domain Identity Types: Boundary Layers Interfacing External Protocols Database

These rules are ALWAYS ACTIVE for all workspace crates and modules defining interfaces for message queuing, persistent storage retrieval, message drafting, and analytical sampling, and internal service boundaries passing email entity references between storage and orchestration components.

### Rules

- **R-DOMAIN-001** SHOULD: Boundary layers interfacing with external protocols or database serialization SHOULD perform conversion to and from mail_domain::MessageId immediately at the ingress and egress points.

### Verify

```bash
# Discover the workspace build harness and execute the complete test suite to confirm type checking and interface compatibility
cargo test --workspace
# Discover the repository static analysis and linting configuration to verify public contracts adhere to domain identifier typing standards
cargo clippy --workspace --all-targets
```

**Accept when:**
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>