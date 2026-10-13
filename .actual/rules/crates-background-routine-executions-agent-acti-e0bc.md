# Adoption of openagc-core Modular Domain Orchestration Library: Background Routine Executions Agent Actions Side

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, and agent interaction coordinators.

### Rules

- **R-COR-001** MUST: Background routine executions and agent actions with side effects MUST register pending requests and evaluate execution constraints through Approvals before performing state alterations.

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
Claude Code MUST NOT skip or defer verification. Verification is mandatory for all domain persistence or background routine modifications.
</enforcement>