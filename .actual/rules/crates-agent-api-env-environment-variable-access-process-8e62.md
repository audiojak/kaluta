# std::env Environment Variable Access in Process Execution: Process Locator Implementations Provide Explicit Search

These rules are ALWAYS ACTIVE for process execution modules, binary locator utilities, and subprocess execution wrappers that query host environment variables or spawn external executables.

### Rules

- **R-ENV-001** SHOULD: Process locator implementations SHOULD provide explicit search path overrides to facilitate isolated testing without mutating host environment variables.

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