# Adoption of openagc-core Modular Domain Orchestration Library: Application Adopt Openagc Core Central Domain

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, agent interaction coordinators, and internal modules requiring access to account-scoped data stores and transactional persistence.

### Rules

- **R-OC-001** MUST: The application MUST adopt openagc-core as the central domain orchestration library, funneling all account lifecycle management, background routine execution, agent action approvals, and persistence through the unified Core engine coordinator.

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
Claude Code MUST NOT skip or defer verification. Automated continuous integration test suites and architectural peer code reviews validate core concurrency, transaction safety, and account isolation.
</enforcement>