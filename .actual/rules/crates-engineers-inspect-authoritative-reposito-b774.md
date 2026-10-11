# agent-api Provider Contract Interface Adoption: Engineers Inspect Authoritative Repository Lock Artifact

These rules are ALWAYS ACTIVE for development of external agent provider integrations, workspace packages defining or consuming agent process lifecycle contracts, and asynchronous agent session orchestration and approval dispatch.

### Rules

- **R-AGNT-001** MUST: Engineers MUST inspect the authoritative repository lock artifact to verify the exact resolved dependency versions of the workspace contract modules and shared libraries before developing or updating agent integrations.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations
# Execute static analysis and contract verification checks defined in the repository workflow configuration
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by continuous integration test passes, architectural code review, and blocking pull requests bypassing agent-api contracts.
</enforcement>