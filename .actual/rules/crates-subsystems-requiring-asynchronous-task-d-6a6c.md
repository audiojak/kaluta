# Tokio Asynchronous Concurrency Model and Runtime Dispatch: Subsystems Requiring Asynchronous Task Dispatch Outside

These rules are ALWAYS ACTIVE for all asynchronous services, routine schedulers, agent session runners, and database transactional routines within the workspace.

### Rules

- **R-TO-001** MUST: Subsystems requiring asynchronous task dispatch from outside an active asynchronous context MUST obtain and use the shared runtime handle rather than constructing independent runtime instances.

### Verify

```bash
# Discover and run the repository build script or test suite to verify asynchronous runtime task completion
# Execute the workspace linting and static analysis tool to ensure all asynchronous call sites adhere to non-blocking concurrency guidelines
```

**Accept when:**
- All asynchronous runtime unit and integration tests pass without executor panic or task starvation.
- Static concurrency analysis verifies that blocking operations are isolated to dedicated blocking tasks.
- External thread caller invocations execute against the encapsulated runtime without leaking unmanaged threads.

<enforcement>
Claude Code MUST NOT skip or defer verification. All asynchronous runtime instantiations and blocking logic are strictly enforced via automated continuous integration and static analysis checks.
</enforcement>