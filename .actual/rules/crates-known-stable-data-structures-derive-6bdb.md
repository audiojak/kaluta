# Adoption of serde_json for Dynamic JSON Interchange and Parsing: Known Stable Data Structures Derive Formal

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-SERDE-001** SHOULD: Known stable data structures SHOULD derive formal deserialization schemas rather than relying entirely on untyped serde_json::Value traversals.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite governing core guide transformations and agent stream decoding.
# Locate and run the static analysis and linter checks to verify correct error handling on JSON parsing routines.
```

**Accept when:**
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>