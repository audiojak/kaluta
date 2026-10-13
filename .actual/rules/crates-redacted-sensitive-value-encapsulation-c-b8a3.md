# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Consumers Inspect Workspace Dependency Configuration Project

These rules are ALWAYS ACTIVE for modules handling authentication credentials, tokens, or cryptographic verifiers across core engine and provider interface crates.

### Rules

- **R-RED-001** MUST: Consumers MUST inspect the workspace dependency configuration and project lock artifact to resolve and bind the exact version of the mail_domain library before referencing its types.
- **R-RED-002** MUST: Constructors and contract factories MUST accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.
- **R-RED-003** MUST: Invocations of expose MUST remain strictly localized to transport adapters during outbound network transmission or credential persistence.

### Verify

```bash
# Discover the project test runner and execute the automated test suite covering secret storage and authentication workflows
# Run repository static analysis tools to verify that credential structures wrap sensitive fields within mail_domain::Redacted
# Execute repository search directives to verify that invocations of expose remain confined to transport adapters
```

**Accept when:**
- All unit and integration tests across core engine and provider modules pass without errors.
- Public interfaces accepting tokens or secrets strictly require mail_domain::Redacted rather than primitive strings.
- No unauthorized unwrap calls are detected outside designated transport adapters.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>