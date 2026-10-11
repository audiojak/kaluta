# tracing-subscriber Structured Diagnostic Registry and Event Layering: Subscriber Layers Recording Structured Event Fields

These rules are ALWAYS ACTIVE for all background services, runtime engines, and agent provider implementations that produce diagnostic events or telemetry, and all custom subscriber layers and diagnostic sinks consuming structured events.

### Rules

- **R-TR-001** MUST: All subscriber layers recording structured event fields MUST evaluate sensitive attributes through redaction visitors to prevent credential and identifier leakage.

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
Claude Code MUST NOT skip or defer verification. All new diagnostic call sites must adhere to structured field conventions and pass automated continuous integration test suites verifying event listener delivery and field redaction behavior.
</enforcement>