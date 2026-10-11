# Adoption of serde_json for Dynamic JSON Interchange and Parsing: System Adopt Serde Json Dynamic Document

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-SERDE-001** MUST: The system MUST adopt serde_json for dynamic document modeling, serialization, and deserialization of untyped or variable JSON structures.
- **R-SERDE-002** MANDATORY: Execute the lock-version grounding process before writing code that uses a versioned library (discover manifest, identify build tool, inspect lock artifact, look up official documentation for exact version, confirm API existence, re-run per dependency at point of use).
- **R-SERDE-003** MANDATORY: Apply defensive accessors and pattern matching when traversing dynamic value objects to safely handle missing or mismatched types.
- **R-SERDE-004** MANDATORY: Provide concise single-line summarization when converting multi-line or expansive payload structures into audit and event streams.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite governing core guide transformations and agent stream decoding.
cargo test
# Locate and run the static analysis and linter checks to verify correct error handling on JSON parsing routines.
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

<enforcement>
Claude Code MUST NOT skip or defer verification. Pull requests introducing ad-hoc JSON parsers or unhandled unwraps on dynamic data trees will be blocked until corrected.
</enforcement>