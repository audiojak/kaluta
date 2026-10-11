# tokio::process Asynchronous Command Execution: Child Process Completion Awaited Wait Output

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes, system tools, or asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-PROC-001** SHOULD: Child process completion SHOULD be awaited using `child.wait_with_output()` for complete batch results or handled via explicit stream readers with cancellation grace intervals for streaming sessions.
- **R-PROC-002** EXCEPTION: Synchronous child process execution is permitted only in standalone test mock fixtures that run outside the asynchronous executor context (EXC-20-001).

### Verify

```bash
# Discover the workspace test runner from the project manifest and execute target validating process execution
cargo test --test process_execution

# Execute workspace static analysis to verify no blocking standard process primitives are invoked in async contexts
cargo check --all-targets

# Inspect the lock artifact in the repository root to verify runtime dependency matches pinned resolution
git diff --check Cargo.lock
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests introducing external process interaction are verified via automated test execution, static code analysis rules forbidding synchronous process execution calls in asynchronous source modules, and peer code review.
</enforcement>