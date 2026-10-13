# openagc-core Internal Library Module Adoption for Core Application Orchestration: Mailbox Mutations Label Updates Executed Through

These rules are ALWAYS ACTIVE for all domain business logic, mailbox mutation workflows, scheduled routine orchestrations, external agent connectors, and multi-account synchronization lifecycle management.

### Rules

- **R-MAIL-001** MUST: Mailbox mutations and label updates executed through Core MUST generate reversible action tokens to support atomic undo and redo operations across synchronized stores.

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
Claude Code MUST NOT skip or defer verification. All mailbox mutations and label updates must execute through the core module, generate reversible action tokens, and pass automated unit, integration, and static analysis checks.
</enforcement>