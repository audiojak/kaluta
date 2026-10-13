# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Test Fixtures Requiring External Executable Execution

These rules are ALWAYS ACTIVE for modules responsible for agent provider integrations, core service and routine orchestration workflows that coordinate external agent runners or cloud routine proxies.

### Rules

- **R-LOC-001** SHOULD: Test fixtures requiring external executable execution SHOULD configure isolated Locator instances targeting temporary directories with deterministic mock executables rather than relying on global system binaries.

### Verify

```bash
# Discover workspace dependency manifest and execute test runner
cargo test --workspace
# Execute project verification test suite for cloud routine runners
cargo test --test routine_runner
# Run static linting and dependency analysis checks
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory via continuous integration test pipelines and peer code review.
</enforcement>