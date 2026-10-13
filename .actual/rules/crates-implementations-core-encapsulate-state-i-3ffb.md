# openagc-core Internal Library Module Adoption for Core Application Orchestration: Implementations Core Encapsulate State Instantiation Within

These rules are ALWAYS ACTIVE for all domain business logic, mailbox mutation workflows, scheduled routine orchestrations, external agent connectors, and multi-account synchronization lifecycle management.

### Rules

- **R-OC-001** MUST: Implementations of Core MUST encapsulate state instantiation within a validated configuration structure and propagate asynchronous domain events via an EventListener contract.

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
Claude Code MUST NOT skip or defer verification. Violations will be blocked during automated verification and architectural boundary linting.
</enforcement>