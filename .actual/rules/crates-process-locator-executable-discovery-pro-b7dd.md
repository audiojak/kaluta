# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Modules Provide Factory Constructors That Initialize

These rules are ALWAYS ACTIVE for modules responsible for agent provider integrations that invoke or detect external command-line processes, and core service and routine orchestration workflows that coordinate external agent runners or cloud routine proxies.

### Rules

- **R-LOC-001** MAY: Modules MAY provide factory constructors that initialize default Locator configurations for standard runtime environments when explicit locator override is not provided.

### Verify

```bash
# Discover the workspace dependency manifest and execute the test runner to verify all provider detection and session startup routines pass.
cargo test --workspace

# Execute the project verification test suite for cloud routine runners to confirm isolated locator behavior with simulated executables.
cargo test --test cloud_routine_runner

# Run static linting and dependency analysis checks to verify no direct process spawning occurs outside the locator abstraction.
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

<enforcement>
Claude Code MUST NOT skip or defer verification. All external process interactions must utilize the locator module abstraction.
</enforcement>