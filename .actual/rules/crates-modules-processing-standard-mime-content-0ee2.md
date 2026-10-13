# Adoption of base64 Engine Architecture for Protocol Serialization: Modules Processing Standard Mime Content Email

These rules are ALWAYS ACTIVE for modules processing standard MIME content, email body payloads, and IMAP transport data.

### Rules

- **R-MIME-001** MUST: Modules processing standard MIME content, email body payloads, and IMAP transport data MUST use the base64::engine::general_purpose::STANDARD engine instance.

### Verify

```bash
# Discover workspace build script and run static compilation to verify all base64 method invocations resolve against the Engine trait.
cargo check --workspace

# Discover and run the workspace test runner to verify encoding and decoding round-trip fixtures across mail and provider suites.
cargo test --workspace

# Discover and execute repository linting suites to ensure no prohibited free functions or unapproved encoding libraries are introduced.
cargo clippy --workspace -- -D warnings
```

**Accept when:**
- All binary-to-text and text-to-binary conversions in synchronization and provider modules compile cleanly using Engine trait methods.
- All unit and integration tests covering MIME header composition, IMAP body fetching, and OAuth token exchanges pass without assertion failures.
- Static analysis detects zero invocations of legacy or unapproved base64 free functions.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>