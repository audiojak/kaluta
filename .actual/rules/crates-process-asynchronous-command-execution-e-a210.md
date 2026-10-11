# tokio::process Asynchronous Command Execution: External System Process Invocations Use Tokio

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes or system tools, and all asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-PROC-001** MUST: All external system process invocations MUST use tokio::process::Command rather than synchronous blocking process primitives to ensure non-blocking asynchronous task execution.

### Verify

```bash
# Discover the workspace test runner from the project manifest and execute the test target validating process execution and timeout behavior.
cargo test --test process_tests

# Execute the workspace static analysis and linting suite to verify that no blocking standard process primitives are invoked within asynchronous execution contexts.
cargo clippy --all-targets -- -D warnings

# Inspect the lock artifact in the repository root to verify that the asynchronous runtime dependency matches the pinned resolution specification.
git diff --exit-code Cargo.lock
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification. All external process execution must strictly adhere to asynchronous Tokio primitives.
</enforcement>