# Testing Framework Adoption for Test Suite Isolation: Test Suites Adopt Testing Framework Structured

These rules are ALWAYS ACTIVE for unit and integration test suites covering core client routines, UI shell components, and agent coordination panels.

### Rules

- **R-TEST-001** MUST: Test suites MUST adopt the Testing framework for structured unit and integration verification, ensuring all test suites execute within isolated test boundaries.
- **R-TEST-002** MUST: Initialize scratch directory allocations during test setup and ensure isolation paths are completely decoupled from production configuration paths.
- **R-TEST-003** MUST: Verify that test keychain service identifiers utilize distinct domain prefixes to avoid collision with production credentials.
- **R-TEST-004** MANDATORY (DISCOVERY POLICY): Omit all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.
- **R-TEST-005** MANDATORY (LOCK-VERSION GROUNDING): Before writing code that uses a versioned library, execute in order: (1) Find the dependency manifest in the repo. (2) Identify the build tool from the manifest. (3) Inspect the repository lock or resolution artifact to determine the exact resolved version. (4) Look up official docs/changelogs/public API reference for that exact version. (5) Confirm every API, class, or function exists in that exact version's documentation before using it. (6) Re-run steps 3-5 per dependency at point of use for version-sensitive behavior.

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
Claude Code MUST NOT skip or defer verification. Automated continuous integration pipeline and peer code review audit new test suites for adherence to test isolation standards. Pull requests containing tests that access real data directories or omit scratch isolation MUST be blocked from merging.
</enforcement>