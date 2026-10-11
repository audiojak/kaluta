# Adoption of openagc-core Modular Domain Orchestration Library: Before Implementing Updating Components That Depend

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, agent interaction coordinators, and internal modules requiring access to account-scoped data stores and transactional persistence.

### Rules

- **R-OPENAGC-001** MUST: Before implementing or updating components that depend on openagc-core or related workspace modules, the consumer MUST inspect the repository dependency lock artifact to determine and adhere to the exact resolved versions.

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
Claude Code MUST NOT skip or defer verification. Architectural and peer code reviews enforce compliance, and changes bypassing the core coordinator or omitting transactional closures must be rejected.
</enforcement>