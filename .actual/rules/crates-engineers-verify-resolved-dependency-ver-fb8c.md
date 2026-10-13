# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Engineers Verify Resolved Dependency Version Serde

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax trees, configuration schemas, persistent metadata records, and inter-component transfer payloads.

### Rules

- **R-SERDE-001** MUST: Engineers MUST verify the resolved dependency version for Serde against the project lock artifact before introducing or updating derived domain structures.

### Verify

```bash
# Discover the project manifest and lock artifact, inspect resolved dependency version for Serde,
# then run the project compiler verification tool and test runner:
# (Example discovery workflow: locate Cargo.toml / Cargo.lock or equivalent manifest/lock, run cargo check / cargo test)
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory via automated CI checks, compiler validation, unit test suites, and peer code reviews.
</enforcement>