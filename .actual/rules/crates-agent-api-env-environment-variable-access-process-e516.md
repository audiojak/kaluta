# std::env Environment Variable Access in Process Execution: Command Execution Invocations Enforce Explicit Timeouts

These rules are ALWAYS ACTIVE for process execution modules, binary locator utilities, and subprocess execution wrappers that spawn external executables or interact with host environment variables.

### Rules

- **R-ENV-001** MUST: Command execution invocations MUST enforce explicit timeouts on child process completion to prevent resource exhaustion from hanging executions.

### Verify

```bash
# Discover the repository build and test script and run test suites covering process execution and path resolution
# Discover the repository static analysis suite and verify that environment variable queries are restricted to locator modules
```

**Accept when:**
- All automated process execution tests pass with explicit timeout verification and isolated locator paths.
- Static analysis confirms no unencapsulated environment variable queries exist outside approved locator modules.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>