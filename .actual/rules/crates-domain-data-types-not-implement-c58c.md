# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Domain Data Types Not Implement Bespoke

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax trees, configuration schemas, persistent metadata records, and inter-component transfer payloads shared across service layers, storage layers, or protocol boundaries.

### Rules

- **R-SERDE-001** MUST_NOT: Domain data types MUST NOT implement bespoke manual string parsers or hand-written serialization logic when equivalent representations can be derived through Serde traits.

### Verify

```bash
# Discover and run package manager test runner for domain serialization
# Discover and run compiler verification tool for macro trait bounds
cargo test
cargo check --all-targets
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>