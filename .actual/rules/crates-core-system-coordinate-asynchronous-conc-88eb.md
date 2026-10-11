# Tokio Asynchronous Concurrency Model and Runtime Dispatch: Core System Coordinate Asynchronous Concurrency Operations

These rules are ALWAYS ACTIVE for all asynchronous services, routine schedulers, agent session runners, and database transactional routines within the workspace.

### Rules

- **R-TOKIO-001** MUST: The core system MUST coordinate asynchronous concurrency operations through the adopted Tokio runtime using centralized runtime dispatch, preventing uncontrolled executor fragmentation across modules.

### Verify

```bash
# Discover and run the repository build/test script to verify asynchronous runtime task completion
# Run workspace linting and static analysis to ensure adherence to non-blocking concurrency guidelines
```

**Accept when:**
- All asynchronous runtime unit and integration tests pass without executor panic or task starvation.
- Static concurrency analysis verifies that blocking operations are isolated to dedicated blocking tasks.
- External thread caller invocations execute against the encapsulated runtime without leaking unmanaged threads.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory.
</enforcement>