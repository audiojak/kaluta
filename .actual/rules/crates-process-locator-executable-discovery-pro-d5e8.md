# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Before Implementing Changes Additions That Consume

These rules are ALWAYS ACTIVE for modules responsible for agent provider integrations that invoke or detect external command-line processes, and core service and routine orchestration workflows that coordinate external agent runners or cloud routine proxies.

### Rules

- **R-LOC-001** MUST: Before implementing changes or additions that consume versioned dependency modules, the consumer MUST inspect the project lock and resolution artifact to resolve and verify the exact dependency version.
- **R-LOC-002** MUST: Utilize `agent_api::process::Locator` for all external executable discovery and process resolution instead of direct ambient system environment path lookup or hardcoded executable lookups.
- **R-LOC-003** MUST: Construct Locator instances during application bootstrap and inject shared references into provider factories and detection routines.

### Verify

```bash
# Discover workspace dependency manifest and execute the test runner
workspace_manifest=$(find . -name "Cargo.toml" | head -n 1)
if [ -n "$workspace_manifest" ]; then
  cargo test --workspace
else
  echo "Manifest not found"
  exit 1
fi

# Run static linting and dependency analysis checks to verify no direct process spawning occurs outside locator abstraction
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

<enforcement>
Claude Code MUST NOT skip or defer verification. All external process interactions must utilize the locator module abstraction and comply with lock-version grounding requirements.
</enforcement>