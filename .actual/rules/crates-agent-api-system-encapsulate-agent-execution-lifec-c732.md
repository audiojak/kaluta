# AgentManager Realtime Session Event Routing and Coordination Boundary: System Encapsulate Agent Execution Lifecycle Operations

These rules are ALWAYS ACTIVE for all modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

### Rules

- **R-AGT-001** MUST: The system MUST encapsulate all agent execution lifecycle operations and multi-turn dispatching behind the AgentManager coordination boundary.
- **R-AGT-002** MANDATORY (DISCOVERY POLICY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.
- **R-AGT-003** MANDATORY (LOCK-VERSION GROUNDING): Before writing code that uses a versioned library, execute in order: (1) Find dependency manifest, (2) Identify build tool, (3) Inspect repository lock/resolution artifact, (4) Look up official docs for that exact version, (5) Confirm every API/class/function exists in that version, (6) Re-run steps 3-5 per dependency at point of use.
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
Claude Code MUST NOT skip or defer verification. All pull requests introducing uncoordinated direct provider invocations will be blocked by automated checks. Code introducing unbuffered channel leaks or unsynchronized shared mutable state will be rejected during peer review.
</enforcement>