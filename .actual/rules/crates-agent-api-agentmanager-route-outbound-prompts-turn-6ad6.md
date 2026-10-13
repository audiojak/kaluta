# AgentManager Realtime Session Event Routing and Coordination Boundary: Agentmanager Route Outbound Prompts Turn Executions

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGM-001** MUST: AgentManager MUST route all outbound prompts and turn executions asynchronously using session identifiers to isolate concurrent session state.

### Verify

```bash
# Discover and execute test runner for session lifecycle management and concurrent event streaming
# Execute repository static analysis and linting suite to confirm concurrency lock scopes and channel sender patterns
```

**Accept when:**
- All unit and integration tests covering session creation, turn dispatching, event reception, and cancellation pass without error.
- Realtime events received on the channel receiver match expected event sequences and correlate accurately with initiating session identifiers.
- Session registries accurately reflect zero active sessions following explicit session closure and cancellation operations.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated test suites and static analysis gates verify adherence to session coordination boundaries.
</enforcement>