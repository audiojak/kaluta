# agent-api Provider Contract Interface Adoption: Agent Providers Propagate Execution Events Status

These rules are ALWAYS ACTIVE for development of external agent provider integrations, workspace packages defining or consuming agent process lifecycle contracts, and asynchronous agent session orchestration and approval dispatch.

### Rules

- **R-AGENT-001** MUST: Agent providers MUST propagate execution events, status transitions, and user approval interactions using the standardized AgentEvent contract.
- **R-AGENT-002** MANDATORY: Implement new agent providers in isolated workspace packages that declare a dependency on the agent-api contract module.
- **R-AGENT-003** MANDATORY: Ensure all provider event sinks handle AgentEvent streams asynchronously without blocking the parent execution thread.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations
# Execute static analysis and contract verification checks defined in the repository workflow configuration
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration test passes verifying contract implementations across workspace packages are required. Pull requests bypassing agent-api contracts or directly invoking provider subprocesses in core will be blocked during code review.
</enforcement>