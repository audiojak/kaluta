# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Provider Implementations Routine Runners Not Perform

These rules are ALWAYS ACTIVE for modules responsible for agent provider integrations that invoke or detect external command-line processes, and core service and routine orchestration workflows that coordinate external agent runners or cloud routine proxies.

### Rules

- **R-LOC-001** MUST_NOT: Provider implementations and routine runners MUST NOT perform ambient filesystem path traversal or invoke host executables directly without delegating resolution through an injected Locator instance.

### Verify

```bash
# Discover workspace dependency manifest and execute test runner for provider detection and session startup routines
cargo test --workspace

# Execute verification test suite for cloud routine runners
cargo test --test routine_runner_tests

# Run static linting and dependency analysis checks to verify no direct process spawning occurs
cargo clippy --all-targets -- -D warnings
```

**Accept when:**
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

<enforcement>
Claude Code MUST NOT skip or defer verification. All external process interactions must utilize the locator module abstraction and comply with mandatory verification steps.
</enforcement>