# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Components Handling Sensitive Credential Data Including

These rules are ALWAYS ACTIVE for all files handling authentication credentials, tokens, or cryptographic verifiers across core engine and provider interface crates.

### Rules

- **R-RED-001** MUST: Components handling sensitive credential data, including OAuth tokens, client secrets, PKCE verifiers, and authorization codes, MUST encapsulate values within the mail_domain::Redacted wrapper type at interface boundaries rather than passing raw string primitives.
- **R-RED-002** MUST: Invocations of the unwrap or expose method on mail_domain::Redacted MUST remain strictly localized to transport adapters during outbound network transmission or credential persistence.
- **R-RED-003** MUST: Constructors and contract factories accepting sensitive credentials MUST accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.

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
Claude Code MUST NOT skip or defer verification. Pull requests passing raw secret strings across contract boundaries are blocked until updated with wrapper types, and unauthorized expose calls outside transport layers are flagged for refactoring.
</enforcement>