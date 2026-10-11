# tokio::process Asynchronous Command Execution: Implementations Not Invoke Synchronous Blocking Wait

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes or system tools, and all asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-PROC-001** MUST_NOT: Implementations MUST_NOT invoke synchronous blocking wait operations on child processes within asynchronous executor threads.

### Verify

```bash
# Discover the workspace test runner from the project manifest and execute the test target validating process execution and timeout behavior.
cargo test --test process_tests

# Execute the workspace static analysis and linting suite to verify that no blocking standard process primitives are invoked within asynchronous execution contexts.
cargo clippy --all-targets --all-features
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated verification and peer code review will enforce these rules at continuous integration.
</enforcement>