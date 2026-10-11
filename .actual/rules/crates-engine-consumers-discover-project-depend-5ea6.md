# Adoption of base64 Engine Architecture for Protocol Serialization: Engine Consumers Discover Project Dependency Manifest

These rules are ALWAYS ACTIVE for all binary payload encoding, decoding, IMAP protocol test harnesses, wire serialization utilities, and OAuth identity token parsing, PKCE exchange generation, and authentication parameter formatting.

### Rules

- **R-BASE64-001** MUST: Engine consumers MUST discover the project dependency manifest and resolved lock artifact to determine the authoritative release before writing or modifying base64 encoding code.

### Verify

```bash
# Discover workspace build script and run static compilation to verify all base64 method invocations resolve against the Engine trait.
# Discover and run the workspace test runner to verify encoding and decoding round-trip fixtures across mail and provider suites.
# Discover and execute repository linting suites to ensure no prohibited free functions or unapproved encoding libraries are introduced.
```

**Accept when:**
- All binary-to-text and text-to-binary conversions in synchronization and provider modules compile cleanly using Engine trait methods.
- All unit and integration tests covering MIME header composition, IMAP body fetching, and OAuth token exchanges pass without assertion failures.
- Static analysis detects zero invocations of legacy or unapproved base64 free functions.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration builds fail upon detecting compiler warnings or deprecated base64 method invocations.
</enforcement>