# Testing Framework Adoption for Test Suite Isolation: Tests Requiring Credential Storage Declare Dedicated

These rules are ALWAYS ACTIVE for all unit and integration test suites covering core client routines, UI shell components, and agent coordination panels requiring credential storage.

### Rules

- **R-ISO-001** MUST: Tests requiring credential storage MUST declare dedicated test keychain services and MUST NOT interact with host user keychain services.

### Verify

```bash
# Discover the test execution command from the project manifest and execute the complete automated test suite
# (Fill in based on project manifest, e.g., npm test / cargo test / pytest)
```

**Accept when:**
- All automated test suites execute successfully within isolated scratch environments without accessing real data directories.
- No test cases fail due to state contamination from prior test runs or concurrent test executions.
- All credential operations in test suites resolve against dedicated test keychain namespaces without modifying host user credentials.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory via automated continuous integration pipeline and peer code review auditing new test suites for adherence to test isolation standards.
</enforcement>