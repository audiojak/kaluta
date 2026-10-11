# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Data Structures Persisting Caching Authorization Credentials

These rules are ALWAYS ACTIVE for all authentication flows, provider integrations, core engine crates, and modules handling sensitive credentials, tokens, or cryptographic verifiers.

### Rules

- **R-RED-001** SHOULD: Data structures persisting or caching authorization credentials in memory SHOULD store mail_domain::Redacted instances rather than unpacked scalar strings.
- **R-RED-002** MANDATORY: Expose calls MUST remain localized to transport adapters during outbound network transmission or credential persistence.
- **R-RED-003** MANDATORY: Constructors and contract factories MUST accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.

### Verify

```bash
# Discover and execute the project test runner for secret storage and authentication workflows
# Run repository static analysis tools to verify credential structures wrap sensitive fields within mail_domain::Redacted
# Execute repository search directives to verify invocations of expose remain confined to transport adapters
```

**Accept when:**
- All unit and integration tests across core engine and provider modules pass without errors.
- Public interfaces accepting tokens or secrets strictly require mail_domain::Redacted rather than primitive strings.
- No unauthorized unwrap calls are detected outside designated transport adapters.

<enforcement>
Claude Code MUST NOT skip or defer verification. Pull requests passing raw secret strings across contract boundaries are blocked until updated with wrapper types, and unauthorized expose calls outside transport layers are flagged for refactoring.
</enforcement>