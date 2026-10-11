# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Domain Entities Value Objects Query Structures

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax tree structures shared across service layers, storage layers, or protocol boundaries, configuration schemas, persistent metadata records, and inter-component transfer payloads.

### Rules

- **R-SERDE-001** MUST: Domain entities, value objects, and query structures that cross component or persistence boundaries MUST implement serialization and deserialization capabilities using Serde derive attributes rather than manual serialization routines.

### Verify

```bash
# Discover the project test runner from the project manifest and execute tests covering domain serialization
cargo test --workspace

# Discover the project compiler verification tool and run compilation checks to verify macro trait bounds across all domain modules
cargo check --workspace
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification. All domain data structures crossing boundaries must adopt declarative derive patterns rather than bespoke parsers, enforced by compiler validation and automated continuous integration checks.
</enforcement>