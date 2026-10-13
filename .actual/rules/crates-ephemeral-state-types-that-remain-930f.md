# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Ephemeral State Types That Remain Strictly

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax trees, configuration schemas, persistent metadata records, and inter-component transfer payloads.

### Rules

- **R-SERDE-001** SHOULD_NOT: Ephemeral state types that remain strictly internal to single-function scopes SHOULD NOT implement Serde serialization derives.

### Verify

```bash
# Discover the project test runner from the repository manifest and execute all unit and integration tests covering domain serialization.
# Discover the project compiler verification tool and run compilation checks to verify macro trait bounds across all domain modules.
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration checks executing compiler validation and unit test suites, and peer code reviews verifying that domain data structures adopt declarative derive patterns rather than bespoke parsers.
</enforcement>