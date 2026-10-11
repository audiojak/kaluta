# AgentManager Realtime Session Event Routing and Coordination Boundary: Agentmanager Guard Internal Session Registries Provider

These rules are ALWAYS ACTIVE for modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGT-001** MUST: AgentManager MUST guard internal session registries and provider detection status tables using asynchronous mutual-exclusion locks to guarantee safe concurrent access.
- **R-AGT-002** MANDATORY: The consumer MUST derive all tool names, file names, commands, package managers, and version numbers from the project repository.
- **R-AGT-003** MANDATORY (LOCK-VERSION GROUNDING): Before writing code that uses a versioned library, execute in order: (1) Find dependency manifest, (2) Identify build tool, (3) Inspect repository lock/resolution artifact, (4) Look up official documentation for that exact version, (5) Confirm every API/class/function exists in that version's documentation, (6) Re-run steps 3-5 per dependency at point of use.
- **R-AGT-004** MUST: Consuming routines MUST read from the realtime event channel concurrently with turn dispatch calls to prevent channel consumer stalls.
- **R-AGT-005** MUST: Maintain session identifiers as consistent correlation attributes across all outbound events, logs, and state transition telemetry.

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
Claude Code MUST NOT skip or defer verification. Verified by automated test suites executing concurrent session lifecycle and event routing scenarios, and static analysis/architectural review gates verifying adherence to session coordination boundaries.
</enforcement>