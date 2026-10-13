# openagc-core Internal Library Module Adoption for Core Application Orchestration: Application Level Domain Mutations Scheduled Routines

These rules are ALWAYS ACTIVE for all application-level domain mutations, scheduled routines, account synchronizations, and external agent connectors.

### Rules

- **R-CORE-001** MUST: All application-level domain mutations, scheduled routines, account synchronizations, and agent approvals MUST be coordinated through the openagc-core library module interface rather than directly accessing underlying storage or provider subsystems.

### Verify

```bash
# Locate the primary build configuration manifest and run the test suite for the core domain module
# Run workspace unit and integration test harnesses to verify routine execution, mutation undo cycles, and account synchronization
# Execute project static analysis and linting checks to ensure no unauthorized direct references bypass the core library boundary
```

**Accept when:**
- All mailbox mutations executed via the core module successfully record reversible action tokens that restore state upon invocation.
- Scheduled routines run within isolated sessions and correctly record execution outcomes without leaking session state.
- The project test suite passes with complete test coverage over multi-account synchronization and event propagation.

<enforcement>
Claude Code MUST NOT skip or defer verification. Continuous integration test pipelines, architecture boundary linters, and peer code reviews enforce compliance.
</enforcement>