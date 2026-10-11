# Adoption of base64 Engine Architecture for Protocol Serialization: Modules Providing Higher Level Wire Serialization

These rules are ALWAYS ACTIVE for modules providing higher-level wire serialization, including binary payload encoding/decoding for email body composition/transmission, IMAP protocol test harnesses and wire serialization utilities, and OAuth identity token parsing, PKCE exchange generation, and authentication parameter formatting.

### Rules

- **R-BS64-001** SHOULD: Modules providing higher-level wire serialization SHOULD wrap engine invocations in domain-specific helper functions with explicit error mapping.
- **R-BS64-002** MANDATORY: Bring base64::Engine into module scope whenever calling encode or decode operations on engine instances.
- **R-BS64-003** MANDATORY: Encapsulate recurring engine operations within domain-specific wire helpers to ensure consistent error conversion and avoid repeated engine parameter declarations.
- **R-BS64-004** MANDATORY: Execute lock-version grounding before writing code that uses a versioned library: (1) find dependency manifest, (2) identify build tool, (3) inspect repository lock/resolution artifact for exact resolved version, (4) look up official documentation/changelog/public API reference for that exact version, (5) confirm every API/class/function exists in that version, (6) re-run per dependency at point of use for version-sensitive behavior.

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
Claude Code MUST NOT skip or defer verification. Verification is mandatory.
</enforcement>