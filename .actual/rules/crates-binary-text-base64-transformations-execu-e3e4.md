# Adoption of base64 Engine Architecture for Protocol Serialization: Binary Text Base64 Transformations Execute Through

These rules are ALWAYS ACTIVE for all binary payload encoding/decoding, protocol test harnesses, wire serialization utilities, and OAuth identity token parsing or PKCE exchange generation within the project.

### Rules

- **R-BASE64-001** MUST: All binary-to-text and text-to-binary base64 transformations MUST execute through the base64::Engine trait interface using explicitly configured engine instances.

### Verify

```bash
# Discover the workspace build script and run static compilation to verify all base64 method invocations resolve against the Engine trait.
cargo check --workspace --all-targets

# Discover and run the workspace test runner to verify encoding and decoding round-trip fixtures across mail and provider suites.
cargo test --workspace

# Discover and execute repository linting suites to ensure no prohibited free functions or unapproved encoding libraries are introduced.
cargo clippy --workspace --all-targets -- -D warnings
```

**Accept when:**
- All binary-to-text and text-to-binary conversions in synchronization and provider modules compile cleanly using Engine trait methods.
- All unit and integration tests covering MIME header composition, IMAP body fetching, and OAuth token exchanges pass without assertion failures.
- Static analysis detects zero invocations of legacy or unapproved base64 free functions.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>