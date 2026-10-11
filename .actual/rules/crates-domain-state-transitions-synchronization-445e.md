# Adoption of openagc-core Modular Domain Orchestration Library: Domain State Transitions Synchronization Progress Broadcast

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, and agent interaction coordinators.

### Rules

- **R-ODC-001** SHOULD: Domain state transitions and synchronization progress SHOULD broadcast account-tagged notifications via EventListener implementations utilizing CoreEvent variants.

### Verify

```bash
# Discover the workspace test runner from the project repository configuration and execute the unit and integration test suites validating core engine operations.
# Discover the repository static analysis script and execute verification to confirm compliance with transactional read-write closures.
# Discover and run the multi-account concurrency integration tests to verify account scoping and event isolation.
```

**Accept when:**
- All unit and integration tests covering the core coordinator, account registry, routines, and agent approvals pass without error.
- Static analysis confirms no direct database store access bypasses the core transactional closures.
- Concurrent multi-account test executions demonstrate zero cross-account data leakage and verified event attribution.

<enforcement>
Claude Code MUST NOT skip or defer verification. All changes must be verified via the test suites, static analysis, and multi-account concurrency integration tests.
</enforcement>