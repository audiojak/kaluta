# std::env Environment Variable Access in Process Execution: Execution Locators Not Treat Environment Variables

These rules are ALWAYS ACTIVE for process execution modules and binary locator utilities that query host environment variables and spawn external executables.

### Rules

- **R-ENV-001** MUST_NOT: Execution locators MUST_NOT treat environment variables as trusted sources for sensitive credential material or unvalidated binary dispatch.

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