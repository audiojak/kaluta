# Testing Framework Adoption for Test Suite Isolation: Test Suites Exercising Persistent Storage Runtime

These rules are ALWAYS ACTIVE for all test suites exercising persistent storage, user keychain services, or runtime state across core clients, UI shell windows, and background agent panels.

### Rules

- **R-STOR-001** MUST: All test suites exercising persistent storage or runtime state MUST operate against isolated scratch data directories and MUST NOT access production data stores.

### Verify

```bash
# Discover the test execution command from the project manifest and execute the complete automated test suite
# Inspect test execution logs to verify that all filesystem and credential operations target temporary test scratch locations
```

**Accept when:**
- All automated test suites execute successfully within isolated scratch environments without accessing real data directories.
- No test cases fail due to state contamination from prior test runs or concurrent test executions.
- All credential operations in test suites resolve against dedicated test keychain namespaces without modifying host user credentials.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>