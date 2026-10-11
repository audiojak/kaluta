# tokio::process Asynchronous Command Execution: Every Asynchronous Child Process Execution Wrapped

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes or system tools, and all asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-TOKIOPROC-001** MUST: Every asynchronous child process execution MUST be wrapped with an explicit tokio::time::timeout to enforce bounded execution duration and prevent indefinite task suspension.

### Verify

```bash
# Discover the workspace test runner from the project manifest and execute the test target validating process execution and timeout behavior.
cargo test --test process_timeouts

# Execute the workspace static analysis and linting suite to verify that no blocking standard process primitives are invoked within asynchronous execution contexts.
cargo clippy --all-targets -- -D warnings
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>