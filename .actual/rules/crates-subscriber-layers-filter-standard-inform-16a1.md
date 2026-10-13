# tracing-subscriber Structured Diagnostic Registry and Event Layering: Subscriber Layers Filter Standard Informational Telemetry

These rules are ALWAYS ACTIVE for all background services, runtime engines, and agent provider implementations that produce diagnostic events or telemetry, and all custom subscriber layers and diagnostic sinks consuming structured events.

### Rules

- **R-TRC-001** SHOULD: Subscriber layers SHOULD filter standard informational telemetry to local diagnostic sinks while dispatching warning and error events to external listeners.
- **R-TRC-002** MANDATORY: Execute the discovery policy to determine tool names, file names, commands, package managers, and version numbers from the project repository.
- **R-TRC-003** MANDATORY: Ground lock versions by finding the dependency manifest, identifying the build tool, inspecting the lock or resolution artifact for exact resolved versions, looking up official documentation, confirming APIs exist, and re-running per dependency at point of use.
- **R-TRC-004** MANDATORY: Configure the global or scoped subscriber registry during runtime initialization before spawning background asynchronous tasks.
- **R-TRC-005** MANDATORY: Implement field visitor traits to sanitize sensitive token values and email patterns prior to formatting event records.

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
Claude Code MUST NOT skip or defer verification. Automated continuous integration test suites and peer code reviews verify event listener delivery, field redaction behavior, and adherence to structured field conventions.
</enforcement>