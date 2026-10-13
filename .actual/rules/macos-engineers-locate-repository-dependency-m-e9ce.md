# Testing Framework Adoption for Test Suite Isolation: Engineers Locate Repository Dependency Manifest Authoritative

These rules are ALWAYS ACTIVE for all unit and integration test suites covering core client routines, UI shell components, and agent coordination panels, as well as test harness configurations managing temporary storage, mock keychain services, and simulated agent runtimes.

### Rules

- **R-TEST-001** MUST: Engineers MUST locate the repository dependency manifest and authoritative lock artifact to verify the exact resolved version of the Testing framework before writing or updating test specifications.

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
Claude Code MUST NOT skip or defer verification. Violation handling: Pull requests containing tests that access real data directories or omit scratch isolation MUST be blocked from merging.
</enforcement>