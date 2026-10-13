# agent-api Provider Contract Interface Adoption: Agent Provider Implementations Not Expose Internal

These rules are ALWAYS ACTIVE for external agent provider integrations, workspace packages defining or consuming agent process lifecycle contracts, and asynchronous agent session orchestration and approval dispatch.

### Rules

- **R-AGT-001** MUST_NOT: Agent provider implementations MUST NOT expose provider-internal process handling primitives, argument parsers, or direct process streams outside their respective integration crates.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations
# Execute static analysis and contract verification checks defined in the repository workflow configuration
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by continuous integration test passes verifying contract implementations across workspace packages and architectural code reviews.
</enforcement>