# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Bounded Channels Explicit Capacity Limits Preferred

These rules are ALWAYS ACTIVE for all asynchronous service actors, event broadcasting and coalescing buses, socket and protocol handlers, and agent streaming pipelines.

### Rules

- **R-MPSC-001** SHOULD: Bounded channels with explicit capacity limits SHOULD be preferred over unbounded channels to prevent unbounded memory growth and provide backpressure to senders.
- **R-MPSC-002** MANDATORY: The consumer MUST discover tool names, file names, commands, package managers, and version numbers from the project repository.
- **R-MPSC-003** MANDATORY: Before writing code that uses a versioned library, execute the lock-version grounding sequence: find dependency manifest, identify build tool, inspect repository lock/resolution artifact, look up official documentation for the exact version, and confirm every API/class/function exists in that version.
- **R-MPSC-004** MANDATORY: Instantiate channels using explicit capacity configurations, passing cloned sender handles to producer tasks while retaining the single receiver handle within the dedicated consumer task loop.
- **R-MPSC-005** MANDATORY: Pair channel reception with cancellation-safe select blocks and timeout guards to ensure receiver tasks respond promptly to shutdown signals.

### Verify

```bash
# Discover the workspace test runner and execute all asynchronous unit and integration tests covering channel communication and event coalescing.
# Discover the project static analysis and linting scripts to verify asynchronous channel usage complies with concurrency guidelines.
```

**Accept when:**
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated continuous integration test runs executing concurrency test suites and static analysis checks are mandatory.
</enforcement>