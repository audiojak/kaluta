# Tokio Asynchronous Concurrency Model and Runtime Dispatch: Prior Implementing Updating Tokio Runtime Dependencies

These rules are ALWAYS ACTIVE for all asynchronous services, routine schedulers, agent session runners, and database transactional routines within the workspace.

### Rules

- **R-TOK-001** MUST: Prior to implementing or updating Tokio runtime dependencies, the consumer MUST inspect the repository lock artifact to determine the exact resolved dependency version and adhere strictly to its documented concurrency APIs.
- **R-TOK-002** MUST: Ensure all newly introduced asynchronous background tasks reference the centralized runtime handle rather than constructing separate runtime environments.
- **R-TOK-003** MUST: Apply bounded duration limits to asynchronous child process executions and inter-process communications.

### Verify

```bash
# Discover and run the repository build script or verification command that executes the test suite
# Execute the workspace linting and static analysis tool to ensure all asynchronous call sites adhere to non-blocking concurrency guidelines
```

**Accept when:**
- All asynchronous runtime unit and integration tests pass without executor panic or task starvation.
- Static concurrency analysis verifies that blocking operations are isolated to dedicated blocking tasks.
- External thread caller invocations execute against the encapsulated runtime without leaking unmanaged threads.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration test runs executing asynchronous concurrency test suites, static analysis checks detecting blocking operations inside asynchronous function bodies, and peer code reviews.
</enforcement>