# Testing Framework Adoption for Test Suite Isolation: Test Host Window Instances Not Persist

These rules are ALWAYS ACTIVE for all test suites, UI shell components, and test harness configurations.

### Rules

- **R-TSI-001** MUST_NOT: Test host window instances MUST NOT persist autosave frame names or window positioning state to the host operating system preferences.

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
Claude Code MUST NOT skip or defer verification. Verification is mandatory for all test framework and isolation rule changes.
</enforcement>