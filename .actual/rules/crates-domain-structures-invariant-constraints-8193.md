# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Domain Structures Invariant Constraints Execute Validation

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax trees, configuration schemas, persistent metadata records, and inter-component transfer payloads.

### Rules

- **R-SERDE-001** MUST: Domain structures with domain invariant constraints MUST execute domain validation routines immediately after deserialization before transferring state into domain operations.

### Verify

```bash
# Discover the project test runner from the repository manifest and execute all unit and integration tests covering domain serialization.
# Discover the project compiler verification tool and run compilation checks to verify macro trait bounds across all domain modules.
cargo test
cargo check
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration checks executing compiler validation and unit test suites, as well as peer code reviews.
</enforcement>