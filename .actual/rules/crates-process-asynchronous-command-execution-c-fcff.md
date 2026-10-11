# tokio::process Asynchronous Command Execution: Consumers Locate Repository Dependency Manifest Verify

These rules are ALWAYS ACTIVE for all internal workspace packages and modules executing external child processes or system tools, and all asynchronous service routines managing interactive or background tool sessions.

### Rules

- **R-PROC-001** MUST: Consumers MUST locate the repository dependency manifest and verify the resolved version against the authoritative resolution artifact before adopting or invoking asynchronous process management interfaces.

### Verify

```bash
# Discover the workspace test runner from the project manifest and execute the test target validating process execution and timeout behavior.
# Execute the workspace static analysis and linting suite to verify that no blocking standard process primitives are invoked within asynchronous execution contexts.
# Inspect the lock artifact in the repository root to verify that the asynchronous runtime dependency matches the pinned resolution specification.
```

**Accept when:**
- All test suites exercising process invocation, exit status checking, and timeout cancellation pass without timeouts or deadlocks.
- Static inspection confirms zero usage of blocking process execution primitives inside asynchronous runtime tasks.
- The active dependency tree matches the locked dependency specifications discovered in the project repository.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>