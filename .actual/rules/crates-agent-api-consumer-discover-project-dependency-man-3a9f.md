# AgentManager Realtime Session Event Routing and Coordination Boundary: Consumer Discover Project Dependency Manifest Lock

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGT-001** MUST: The consumer MUST discover the project dependency manifest and lock artifact to determine the exact resolved versions of asynchronous concurrency and messaging components prior to implementation.
- **R-AGT-002** MANDATORY: Execute the lock-version grounding sequence (manifest discovery, build tool identification, lock artifact inspection, documentation look-up, and API verification) before writing code that uses a versioned library.
- **R-AGT-003** MUST: Consuming routines MUST read from the realtime event channel concurrently with turn dispatch calls to prevent channel consumer stalls.
- **R-AGT-004** MUST: Maintain session identifiers as consistent correlation attributes across all outbound events, logs, and state transition telemetry.

### Verify

```bash
# Discover the project test runner and execute the integration suite covering session lifecycle management and concurrent event streaming
# Execute the repository static analysis and linting suite to confirm concurrency lock scopes and channel sender patterns comply with project standards
```

**Accept when:**
- All unit and integration tests covering session creation, turn dispatching, event reception, and cancellation pass without error.
- Realtime events received on the channel receiver match expected event sequences and correlate accurately with initiating session identifiers.
- Session registries accurately reflect zero active sessions following explicit session closure and cancellation operations.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests introducing uncoordinated direct provider invocations or unbuffered channel leaks will be blocked.
</enforcement>