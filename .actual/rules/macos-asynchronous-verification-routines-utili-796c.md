# Testing Framework Adoption for Test Suite Isolation: Asynchronous Verification Routines Utilize Explicit Timeout

These rules are ALWAYS ACTIVE for unit and integration test suites covering core client routines, UI shell components, and agent coordination panels, as well as test harness configurations.

### Rules

- **R-ISO-001** SHOULD: Asynchronous verification routines SHOULD utilize explicit timeout structures and non-blocking wait primitives rather than arbitrary execution delays.

### Verify

```bash
# Discover the test execution command from the project manifest and execute the complete automated test suite to ensure all test suites pass.
# Inspect test execution logs to verify that all filesystem and credential operations target temporary test scratch locations.
```

**Accept when:**
- All automated test suites execute successfully within isolated scratch environments without accessing real data directories.
- No test cases fail due to state contamination from prior test runs or concurrent test executions.
- All credential operations in test suites resolve against dedicated test keychain namespaces without modifying host user credentials.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>