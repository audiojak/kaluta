# Adoption of base64 Engine Architecture for Protocol Serialization: Components Constructing Parsing Web Tokens Authentication

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-BASE64-001** MUST: Components constructing or parsing web tokens, authentication exchange parameters, and OAuth identity payloads MUST use the base64::engine::general_purpose::URL_SAFE_NO_PAD engine instance.
- **R-BASE64-002** MANDATORY: Bring base64::Engine into module scope whenever calling encode or decode operations on engine instances.
- **R-BASE64-003** MANDATORY: Encapsulate recurring engine operations within domain-specific wire helpers to ensure consistent error conversion and avoid repeated engine parameter declarations.
- **R-BASE64-004** MANDATORY: Execute lock-version grounding in order (manifest, build tool, lock artifact, public API docs verification) before writing code that uses a versioned library.

### Verify

```bash
# Discover the workspace build script and run static compilation to verify all base64 method invocations resolve against the Engine trait.
# Discover and run the workspace test runner to verify encoding and decoding round-trip fixtures across mail and provider suites.
# Discover and execute repository linting suites to ensure no prohibited free functions or unapproved encoding libraries are introduced.
```

**Accept when:**
- All binary-to-text and text-to-binary conversions in synchronization and provider modules compile cleanly using Engine trait methods.
- All unit and integration tests covering MIME header composition, IMAP body fetching, and OAuth token exchanges pass without assertion failures.
- Static analysis detects zero invocations of legacy or unapproved base64 free functions.

<enforcement>
Claude Code MUST NOT skip or defer verification. Violation handling: Continuous integration builds fail upon detecting compiler warnings or deprecated base64 method invocations. Pull requests introducing unapproved encoding patterns or free functions are blocked until refactored to approved engine patterns.
</enforcement>