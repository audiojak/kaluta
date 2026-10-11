# Adoption of serde_json for Dynamic JSON Interchange and Parsing: Modules Processing External Payloads Dynamic Schemas

These rules are ALWAYS ACTIVE for modules processing external payloads with dynamic schemas.

### Rules

- **R-EXT-001** MUST: Modules processing external payloads with dynamic schemas MUST utilize serde_json::Value to isolate polymorphic data trees before structural validation.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite governing core guide transformations and agent stream decoding.
# Locate and run the static analysis and linter checks to verify correct error handling on JSON parsing routines.
cargo test
cargo clippy
```

**Accept when:**
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>