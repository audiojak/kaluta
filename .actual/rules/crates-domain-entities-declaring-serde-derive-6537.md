# Serde Derive Macro Adoption for Domain Entity Modeling and Serialization: Domain Entities Declaring Serde Derive Traits

These rules are ALWAYS ACTIVE for domain entities, value objects, abstract syntax trees, configuration schemas, persistent metadata records, and inter-component transfer payloads.

### Rules

- **R-SERDE-001** SHOULD: Domain entities declaring Serde derive traits SHOULD concurrently derive standard structural traits for equivalence, reproduction, and diagnostic inspection to maintain uniform domain model behavior.
- **R-SERDE-002** MANDATORY: The consumer MUST derive build tools, dependency manifests, and exact resolved versions from the project repository following lock-version grounding before writing code using versioned libraries.
- **R-SERDE-003** MANDATORY: Attach Serde derive attributes directly to domain structure and enumeration definitions alongside standard structural traits for equivalence and cloning.
- **R-SERDE-004** MANDATORY: Structure domain boundaries to execute invariant validation immediately after deserialization before transferring state into business logic routines.

### Verify

```bash
# Discover the project test runner and compiler verification tool from the repository manifest
# Run all unit and integration tests covering domain serialization
# Run compilation checks to verify macro trait bounds across all domain modules
```

**Accept when:**
- All domain entity serialization and deserialization test suites pass without regressions.
- All domain types deriving Serde traits compile cleanly without missing trait bound errors.

<enforcement>
Claude Code MUST NOT skip or defer verification. All domain data structures must adopt declarative derive patterns rather than bespoke parsers, and manual serialization boilerplate is blocked from merging.
</enforcement>