# openagc-core Internal Library Module Adoption for Core Application Orchestration: Domain Consumers Not Bypass Core Contract

These rules are ALWAYS ACTIVE for all domain business logic, mailbox mutation workflows, scheduled routine orchestrations, external agent connectors, and multi-account synchronization lifecycle management.

### Rules

- **R-CORE-001** MUST_NOT: Domain consumers MUST_NOT bypass the Core API contract to directly execute transactional writes on backend storage engines.

### Verify

```bash
# Locate the primary build configuration manifest and run the test suite for the core domain module.
# Run the workspace unit and integration test harnesses to verify routine execution, mutation undo cycles, and account synchronization.
# Execute the project static analysis and linting checks to ensure no unauthorized direct references bypass the core library boundary.
```

**Accept when:**
- All mailbox mutations executed via the core module successfully record reversible action tokens that restore state upon invocation.
- Scheduled routines run within isolated sessions and correctly record execution outcomes without leaking session state.
- The project test suite passes with complete test coverage over multi-account synchronization and event propagation.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests bypassing the core module or directly writing to persistence layers will be blocked during automated verification, and architectural violations must be refactored to consume the public Core interface before merging.
</enforcement>