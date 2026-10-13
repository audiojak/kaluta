# Prompt Fingerprint Generation in Agent Routines: Prompt Evaluation Subsystem Compute Deterministic Fingerprints

These rules are ALWAYS ACTIVE for prompt generation and evaluation routines within the agent API component.

### Rules

- **R-PROMPT-001** MUST: The prompt evaluation subsystem MUST compute deterministic prompt fingerprints using the public prompt_fingerprint function to establish digest identity across prompt generation cycles.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite targeting prompt routine verification.
# Discover and run the snapshot review suite to confirm prompt outputs match approved baselines across all runner targets.
```

**Accept when:**
- All prompt generation tests pass and assert equal fingerprints for identical routine inputs.
- Modified routine descriptions produce distinct fingerprint values of exactly sixteen hexadecimal characters.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated unit test execution and peer code review enforce compliance.
</enforcement>