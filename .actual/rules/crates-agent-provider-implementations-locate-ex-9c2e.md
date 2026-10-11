# agent-api Provider Contract Interface Adoption: Agent Provider Implementations Locate External Binaries

These rules are ALWAYS ACTIVE for development of external agent provider integrations, workspace packages defining or consuming agent process lifecycle contracts, and asynchronous agent session orchestration and approval dispatch.

### Rules

- **R-AGT-001** MUST: Agent provider implementations MUST locate external binaries and execution environments using the process Locator contract provided by agent-api.

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
Claude Code MUST NOT skip or defer verification. Continuous integration test passes verifying contract implementations across workspace packages are required, and pull requests bypassing agent-api contracts or directly invoking provider subprocesses in core will be blocked during code review.
</enforcement>