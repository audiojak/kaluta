# AgentManager Realtime Session Event Routing and Coordination Boundary: Agent Provider Detection Status Checks Utilize

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGT-001** SHOULD: Agent provider detection and status checks SHOULD utilize cached status records unless an explicit refresh is requested by the caller.

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
Claude Code MUST NOT skip or defer verification.
</enforcement>