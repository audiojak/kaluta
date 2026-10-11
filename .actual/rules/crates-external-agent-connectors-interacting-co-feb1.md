# openagc-core Internal Library Module Adoption for Core Application Orchestration: External Agent Connectors Interacting Core Domain

These rules are ALWAYS ACTIVE for all domain business logic, mailbox mutation workflows, scheduled routine orchestrations, external agent connectors, and approval workflows interacting with user accounts and mailboxes.

### Rules

- **R-OAGCC-001** SHOULD: External agent connectors interacting with core domain capabilities SHOULD pass through centralized approval gates before resolving destructive or privileged mailbox actions.

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
Claude Code MUST NOT skip or defer verification. Pull requests bypassing the core module or directly writing to persistence layers will be blocked during automated verification.
</enforcement>