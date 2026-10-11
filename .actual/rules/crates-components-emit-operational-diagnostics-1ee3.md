# tracing-subscriber Structured Diagnostic Registry and Event Layering: Components Emit Operational Diagnostics Telemetry Exclusively

These rules are ALWAYS ACTIVE for all background services, runtime engines, and agent provider implementations that produce diagnostic events or telemetry, as well as all custom subscriber layers and diagnostic sinks consuming structured events.

### Rules

- **R-DIAG-001** MUST: Components MUST emit operational diagnostics and telemetry exclusively through structured tracing macros and register sinks via the centralized subscriber registry.

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