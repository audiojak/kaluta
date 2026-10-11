# agent-api Provider Contract Interface Adoption: External Agent Providers Execution Integrations Implement

These rules are ALWAYS ACTIVE for all external agent providers and agent execution integrations within the workspace.

### Rules

- **R-AGT-001** MUST: All external agent providers and agent execution integrations MUST implement the public contract interfaces defined by the agent-api module rather than establishing proprietary integration protocols.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations.
# Execute static analysis and contract verification checks defined in the repository workflow configuration.
cargo test --workspace
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>