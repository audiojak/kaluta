# Risk-classified tool gating for external agent execution: Autonomous Agent Runtimes Not Execute Core

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-AGENT-001** MUST_NOT: Autonomous agent runtimes MUST NOT execute core mailbox state mutations directly without traversing the permissions policy gate.

### Verify

```bash
cargo test -p crates-permissions
cargo test -p crates-agent-mcp
cargo test -p openagc-core -- agents
```

**Accept when:**
- All tools registered in agent execution paths have an explicit risk tier in crates/permissions.
- External approval tools successfully pause execution until approved via approval cards.
- Attempted tool invocations without policy clearance are rejected by the MCP server shim.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>