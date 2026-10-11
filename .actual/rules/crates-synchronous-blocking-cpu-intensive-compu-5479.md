# Tokio Asynchronous Concurrency Model and Runtime Dispatch: Synchronous Blocking Cpu Intensive Computations Not

These rules are ALWAYS ACTIVE for all asynchronous services, routine schedulers, agent session runners, and database transactional routines within the workspace.

### Rules

- **R-TOKYO-001** MUST_NOT: Synchronous, blocking, or CPU-intensive computations MUST NOT execute directly on asynchronous reactor threads and MUST be dispatched using dedicated blocking task mechanisms.

### Verify

```bash
# Discover the repository build script or verification command that executes the test suite, and run it to verify asynchronous runtime task completion.
# Execute the workspace linting and static analysis tool to ensure all asynchronous call sites adhere to non-blocking concurrency guidelines.
```

**Accept when:**
- All asynchronous runtime unit and integration tests pass without executor panic or task starvation.
- Static concurrency analysis verifies that blocking operations are isolated to dedicated blocking tasks.
- External thread caller invocations execute against the encapsulated runtime without leaking unmanaged threads.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests introducing unencapsulated runtime instantiations or unmanaged blocking logic on async threads fail review, and static analysis concurrency warnings block continuous integration pipeline completion.
</enforcement>