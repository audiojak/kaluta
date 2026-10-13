# AgentManager Realtime Session Event Routing and Coordination Boundary: Realtime Execution Events Emitted Active Agent

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGT-001** MUST: Realtime execution events emitted by active agent providers MUST be multiplexed through an asynchronous channel sink associating every event with its initiating session identifier.
- **R-AGT-002** MANDATORY: Consume routines MUST read from the realtime event channel concurrently with turn dispatch calls to prevent channel consumer stalls.
- **R-AGT-003** MANDATORY: Maintain session identifiers as consistent correlation attributes across all outbound events, logs, and state transition telemetry.

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
Claude Code MUST NOT skip or defer verification. Automated test suites executing concurrent session lifecycle and event routing scenarios and static analysis/architectural review gates verify adherence. Pull requests introducing uncoordinated direct provider invocations will be blocked by automated checks. Code introducing unbuffered channel leaks or unsynchronized shared mutable state will be rejected during peer review.
</enforcement>