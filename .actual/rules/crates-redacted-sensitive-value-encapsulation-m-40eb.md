# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Modules Not Implement Derive Traits That

These rules are ALWAYS ACTIVE for all modules and files handling authentication flows, provider integrations, sensitive credentials, and public contracts defined across core engine and provider interface crates.

### Rules

- **R-RED-001** MUST_NOT: Modules MUST_NOT implement or derive traits that expose the inner value of mail_domain::Redacted instances in telemetry, standard error output, or persistent debug traces.
- **R-RED-002** MUST: Constructors and contract factories MUST accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.
- **R-RED-003** MUST: `expose` calls MUST remain localized to transport adapters during outbound network transmission or credential persistence.

### Verify

```bash
# Discover the project test runner and execute the automated test suite covering secret storage and authentication workflows.
# Run repository static analysis tools to verify that credential structures wrap sensitive fields within mail_domain::Redacted.
# Execute repository search directives to verify that invocations of expose remain confined to transport adapters.
```

**Accept when:**
- All unit and integration tests across core engine and provider modules pass without errors.
- Public interfaces accepting tokens or secrets strictly require mail_domain::Redacted rather than primitive strings.
- No unauthorized unwrap calls are detected outside designated transport adapters.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>