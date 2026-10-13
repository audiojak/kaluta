# Tokio Asynchronous Concurrency Model and Runtime Dispatch: Database Transactions Storage Access Operations Coordinate

These rules are ALWAYS ACTIVE for all asynchronous services, routine schedulers, agent session runners, and database transactional routines within the workspace.

### Rules

- **R-TOK-001** MUST: Database transactions and storage access operations MUST coordinate through asynchronous transactional closures dispatched through the runtime runner.

### Verify

```bash
# Discover and execute the repository build script or verification command that executes the test suite
# Execute the workspace linting and static analysis tool to ensure all asynchronous call sites adhere to non-blocking concurrency guidelines
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All asynchronous runtime unit and integration tests pass without executor panic or task starvation.
- Static concurrency analysis verifies that blocking operations are isolated to dedicated blocking tasks.
- External thread caller invocations execute against the encapsulated runtime without leaking unmanaged threads.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>