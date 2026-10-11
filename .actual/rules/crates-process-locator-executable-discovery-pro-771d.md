# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Provider Components Routine Orchestration Modules Requiring

These rules are ALWAYS ACTIVE for all provider components and routine orchestration modules requiring interaction with external command-line executables.

### Rules

- **R-LOC-001** MUST: All provider components and routine orchestration modules requiring interaction with external command-line executables MUST encapsulate executable lookup and resolution through agent_api::process::Locator.

### Verify

```bash
# Discover workspace dependency manifest and execute test runner
# (Commands derived from repository structure)
cargo test --workspace --lib

# Execute project verification test suite for cloud routine runners
cargo test --test routine_runners

# Run static linting and dependency analysis checks
cargo clippy --all-targets -- -D warnings
```

**Accept when:**
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

<enforcement>
Claude Code MUST NOT skip or defer verification. All external process interactions must utilize the locator module abstraction and pass automated unit and integration verification suites.
</enforcement>