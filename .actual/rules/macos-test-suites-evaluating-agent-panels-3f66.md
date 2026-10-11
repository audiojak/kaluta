# Testing Framework Adoption for Test Suite Isolation: Test Suites Evaluating Agent Panels Execution

These rules are ALWAYS ACTIVE for unit and integration test suites covering core client routines, UI shell components, and agent coordination panels.

### Rules

- **R-ISO-001** MUST_NOT: Test suites evaluating agent panels and execution controllers MUST NOT connect to live external agent runtimes or services.

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