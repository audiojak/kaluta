# agent-api Provider Contract Interface Adoption: Consumers Testing Downstream Agent Orchestration Workflows

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-AGT-001** SHOULD: Consumers testing downstream agent orchestration workflows SHOULD use the fake provider implementation from agent-api to configure readiness states and simulated responses without spawning subprocesses.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations
cargo test --workspace
# Execute static analysis and contract verification checks defined in the repository workflow configuration
cargo clippy --workspace --all-targets
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration test passes verifying contract implementations across workspace packages are mandatory, and pull requests bypassing agent-api contracts or directly invoking provider subprocesses in core will be blocked.
</enforcement>