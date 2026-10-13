# tracing-subscriber Structured Diagnostic Registry and Event Layering: Background Session Engines Provider Workers Attach

These rules are ALWAYS ACTIVE for all background services, runtime engines, and agent provider implementations that produce diagnostic events or telemetry, and all custom subscriber layers and diagnostic sinks consuming structured events.

### Rules

- **R-TRA-001** MUST: Background session engines and provider workers MUST attach subscriber handles to the active asynchronous executor to preserve diagnostic context across task boundaries.

### Verify

```bash
# Discover and run the project repository validation scripts that verify diagnostic layer composition and test suite compliance.
# Execute the repository linting and static analysis suite to confirm absence of unredacted credential patterns in diagnostic outputs.
```

**Accept when:**
- All background service operations emit structured events captured by registered subscriber layers.
- Diagnostic verification tests confirm that sensitive tokens and addresses are scrubbed while warnings and errors reach the event bus.
- Repository verification suites and integration tests pass with zero subscriber dispatch panics or deadlocks.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>