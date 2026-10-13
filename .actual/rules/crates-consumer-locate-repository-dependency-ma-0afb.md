# openagc-core Internal Library Module Adoption for Core Application Orchestration: Consumer Locate Repository Dependency Manifest Inspect

These rules are ALWAYS ACTIVE for all domain business logic, mailbox mutation workflows, scheduled routine orchestrations, external agent connectors, and multi-account synchronization lifecycle management.

### Rules

- **R-CORE-001** MUST: The consumer MUST locate the repository dependency manifest and inspect the authoritative lock artifact to verify the exact resolved version of all internal and external dependencies prior to implementation.

### Verify

```bash
# Locate the primary build configuration manifest and run the test suite for the core domain module
# Run workspace unit and integration test harnesses
# Execute project static analysis and linting checks to ensure no unauthorized direct references bypass the core library boundary
```

**Accept when:**
- All mailbox mutations executed via the core module successfully record reversible action tokens that restore state upon invocation.
- Scheduled routines run within isolated sessions and correctly record execution outcomes without leaking session state.
- The project test suite passes with complete test coverage over multi-account synchronization and event propagation.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by continuous integration test pipelines, architecture boundary linters, and peer code review.
</enforcement>