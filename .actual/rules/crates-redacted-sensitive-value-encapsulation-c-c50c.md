# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Callers Requiring Access Underlying Secret Primitive

These rules are ALWAYS ACTIVE for modules handling authentication credentials, tokens, cryptographic verifiers, and public contracts across core engine and provider interface crates.

### Rules

- **R-RED-001** MUST: Callers requiring access to the underlying secret primitive MUST invoke the explicit expose method only at the immediate point of transmission to authenticated remote services.
- **R-RED-002** MUST: Constructors and contract factories MUST accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.

### Verify

```bash
# Discover and execute project test runner for secret storage/auth workflows
# Run repository static analysis to verify credential structures wrap sensitive fields within mail_domain::Redacted
# Execute repository search to verify that invocations of expose remain confined to transport adapters
```

**Accept when:**
- All unit and integration tests across core engine and provider modules pass without errors.
- Public interfaces accepting tokens or secrets strictly require mail_domain::Redacted rather than primitive strings.
- No unauthorized unwrap calls are detected outside designated transport adapters.

<enforcement>
Claude Code MUST NOT skip or defer verification. Violation handling: Pull requests passing raw secret strings across contract boundaries are blocked until updated with wrapper types, and unauthorized expose calls outside transport layers are flagged for refactoring.
</enforcement>