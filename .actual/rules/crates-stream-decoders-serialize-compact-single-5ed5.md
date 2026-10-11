# Adoption of serde_json for Dynamic JSON Interchange and Parsing: Stream Decoders Serialize Compact Single Line

These rules are ALWAYS ACTIVE for serialization and deserialization of external agent communication transcripts, streams, and tool call parameters, as well as parsing and transformation of semi-structured guide import and change documents.

### Rules

- **R-JSON-001** SHOULD: Stream decoders SHOULD serialize compact single-line summaries of nested structures when converting large arguments into event payloads.
- **R-JSON-002** MANDATORY: Execute the discovery policy and lock-version grounding before writing code that uses a versioned library (inspect manifest, identify build tool, inspect lock/resolution artifact, lookup official documentation for exact version, confirm API existence).
- **R-JSON-003** MANDATORY: Apply defensive accessors and pattern matching when traversing dynamic value objects to safely handle missing or mismatched types.
- **R-JSON-004** MANDATORY: Provide concise single-line summarization when converting multi-line or expansive payload structures into audit and event streams.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite governing core guide transformations and agent stream decoding.
# Locate and run the static analysis and linter checks to verify correct error handling on JSON parsing routines.
```

**Accept when:**
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated continuous integration test suites and peer code review audit error propagation and dynamic value traversal across boundary interfaces.
</enforcement>