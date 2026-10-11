# tokio::process Asynchronous Command Execution: Child Process Standard Input Output Error

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes or system tools, and all asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-PROC-001** MUST: Child process standard input, output, and error streams MUST be explicitly configured using standard process I/O descriptors to prevent unbuffered pipe deadlocks.

### Verify

```bash
# Discover workspace test runner from project manifest and execute test target
cargo test --test process_execution

# Execute static analysis and linting to verify no blocking process primitives are invoked
cargo clippy --all-targets -- -D warnings

# Inspect lock artifact to verify dependency matches pinned resolution
git diff Cargo.lock
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>