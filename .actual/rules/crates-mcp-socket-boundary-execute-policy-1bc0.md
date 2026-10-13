# Risk-classified tool gating for external agent execution: Mcp Socket Boundary Execute Policy Decision

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-MCP-001** MUST: The MCP socket boundary MUST execute policy decision verification against the assigned risk tier prior to dispatching any agent operation.

### Verify

```bash
cargo test -p openagc-core --lib agents
cargo test -p agent-mcp
cargo test -p permissions
```

**Accept when:**
- All tools registered in agent execution paths have an explicit risk tier in crates/permissions.
- External approval tools successfully pause execution until approved via approval cards.
- Attempted tool invocations without policy clearance are rejected by the MCP server shim.

<enforcement>
Claude Code MUST NOT skip or defer verification. All tool invocations lacking valid tier assignment or approval are rejected with policy violation errors, and any tool definition failing to implement risk classification bindings causes a CI build failure.
</enforcement>