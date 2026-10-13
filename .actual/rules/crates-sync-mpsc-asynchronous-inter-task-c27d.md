# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Asynchronous Task Coordination Background Event Dispatching

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-MPSC-001** MUST: Asynchronous task coordination, background event dispatching, and socket message routing MUST use tokio::sync::mpsc channels to pass discrete typed messages between producers and consumers.

### Verify

```bash
# Discover the workspace test runner and execute all asynchronous unit and integration tests covering channel communication and event coalescing.
# Discover the project static analysis and linting scripts to verify asynchronous channel usage complies with concurrency guidelines.
```

**Accept when:**
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated continuous integration test runs executing concurrency test suites and static analysis checks verify compliance.
</enforcement>