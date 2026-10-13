# Adoption of mail_domain Core Domain Identity Types: Domain Modules Derive Standard Serialization Formatting

These rules are ALWAYS ACTIVE for all workspace crates and modules defining interfaces for message queuing, persistent storage retrieval, message drafting, and analytical sampling.

### Rules

- **R-DOM-001** MAY: Domain modules MAY derive standard serialization, formatting, and comparison traits on domain types to support seamless diagnostic logging and structured testing fixtures.
- **R-DOM-002** MANDATORY: Construct domain identifiers via their designated constructors or parsing functions at boundary ingress, propagating mail_domain::MessageId through all internal processing pipelines.
- **R-DOM-003** MANDATORY: Implement transparent newtype representations and reference-based lookups to minimize performance overhead during database queries and queue inspections.
- **R-DOM-004** MANDATORY: Before writing code that uses a versioned library, execute in order: (1) Find the dependency manifest in the repo. (2) Identify the build tool from the manifest. (3) Inspect the repository lock or resolution artifact to determine the exact resolved version. (4) Look up official documentation or public API reference for that exact version. (5) Confirm every API, class, or function you will call exists in that exact version's documentation. (6) Re-run steps 3-5 per dependency at point of use for version-sensitive behavior.

### Verify

```bash
# Discover the workspace build harness and execute the complete test suite to confirm type checking and interface compatibility across all crates consuming the domain library.
cargo test --workspace

# Discover the repository static analysis and linting configuration to verify that public contracts adhere to domain identifier typing standards without primitive fallback.
cargo clippy --workspace --all-targets
```

**Accept when:**
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration blocks merge on compile errors or lint violations where raw strings are substituted for domain types, and pull requests violating domain typing conventions must be revised to adopt mail_domain types before merge.
</enforcement>