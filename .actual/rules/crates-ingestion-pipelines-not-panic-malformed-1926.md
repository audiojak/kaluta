# Adoption of serde_json for Dynamic JSON Interchange and Parsing: Ingestion Pipelines Not Panic Malformed Json

These rules are ALWAYS ACTIVE for ingestion pipelines processing external agent transcripts, streaming event protocols, and dynamic guide definition interchange formats.

### Rules

- **R-JSON-001** MUST_NOT: Ingestion pipelines MUST NOT panic on malformed JSON structures and MUST propagate parse failures as typed domain errors.

### Verify

```bash
# Discover and execute the unit test suite governing core guide transformations and agent stream decoding
# Locate and run static analysis and linter checks to verify correct error handling on JSON parsing routines
```

**Accept when:**
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated CI test suites and peer code reviews enforce serialization, merge planning, and stream decoding compliance. Pull requests introducing ad-hoc JSON parsers or unhandled unwraps on dynamic data trees will be blocked until corrected.
</enforcement>