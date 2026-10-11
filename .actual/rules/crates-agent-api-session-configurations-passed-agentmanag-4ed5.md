# AgentManager Realtime Session Event Routing and Coordination Boundary: Session Configurations Passed Agentmanager Define Immutable

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGM-001** MUST: Session configurations passed to AgentManager MUST define immutable execution boundaries including working directory, prompt source, and endpoint bindings for the session lifetime.

### Verify

```bash
# Discover the project test runner and execute the integration suite covering session lifecycle management and concurrent event streaming.
# Execute the repository static analysis and linting suite to confirm concurrency lock scopes and channel sender patterns comply with project standards.
```

**Accept when:**
- All unit and integration tests covering session creation, turn dispatching, event reception, and cancellation pass without error.
- Realtime events received on the channel receiver match expected event sequences and correlate accurately with initiating session identifiers.
- Session registries accurately reflect zero active sessions following explicit session closure and cancellation operations.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests introducing uncoordinated direct provider invocations or unbuffered channel leaks will be blocked.
</enforcement>