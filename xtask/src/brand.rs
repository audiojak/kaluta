//! `cargo xtask check-brand`: the project's old name (OpenAGC, ADR 0017)
//! appears only where it reads what the old name wrote, or in the record
//! of what happened. Anywhere else it is a leftover of the rename.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Files allowed to name the old project, each for a reason.
const ALLOWED: &[(&str, &str)] = &[
    // The record: dated plans, accepted ADRs, the original product spec.
    ("docs/plans/", "historical plans"),
    ("docs/adr/", "accepted decisions, and ADR 0017 on the rename"),
    ("Open-Source Agentic Email Client", "the original product spec"),
    ("README.md", "the note on the name"),
    ("CLAUDE.md", "never delete under the old data folder; the migration"),
    ("docs/SPECIFICATION.md", "the old name, the migration and old config entries"),
    ("docs/rules-server.md", "OPENAGC_RULES_* still read"),
    // Reading what the old name wrote.
    ("crates/rules-crypto/src/lib.rs", "encryption labels are protocol constants"),
    ("crates/mail-store/src/read.rs", "bodies stored with the old schemes and quote class"),
    ("crates/mail-store/tests/mail.rs", "its test"),
    ("crates/provider-agentmail/src/lib.rs", "sends queued under the old outbox header"),
    ("crates/provider-agentmail/src/tests.rs", "its test"),
    ("crates/kaluta-core/src/guide.rs", "guides exported under the old format"),
    ("crates/kaluta-core/src/facts_io.rs", "facts exported under the old format"),
    ("crates/kaluta-core/src/agent_connect.rs", "the agent config entries the app wrote"),
    ("crates/rules-server/src/main.rs", "OPENAGC_RULES_* settings"),
    ("crates/rules-server/tests/cli.rs", "its test"),
    ("macos/Kaluta/App/Migration.swift", "the first launch takes over OpenAGC"),
    ("macos/Kaluta/App/KalutaApp.swift", "runs the migration at launch"),
    // XcodeGen names the group for `..` after the checkout's folder.
    ("macos/Kaluta.xcodeproj/project.pbxproj", "the checkout folder's name"),
    ("macos/KalutaTests/MigrationTests.swift", "its tests"),
    ("macos/Kaluta/Core/KeychainSecretStore.swift", "never empty OpenAGC's items"),
    ("macos/Kaluta/Features/Settings/ConnectAgent.swift", "names the entry OpenAGC wrote"),
    ("macos/KalutaTests/OutsideAgentTests.swift", "its test"),
    ("scripts/test-macos.sh", "isolation watches OpenAGC's preferences too"),
    ("scripts/snapshot.sh", "isolation watches OpenAGC's preferences too"),
    ("scripts/clean-test-scratch.sh", "sweeps scratch the old name left"),
    ("xtask/src/brand.rs", "this check"),
];

pub fn run(root: &Path) -> Result<()> {
    let out = Command::new("git").arg("-C").arg(root).args(["ls-files", "-z"]).output().context("git ls-files")?;
    let mut found = Vec::new();
    for path in out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(path);
        if ALLOWED.iter().any(|(prefix, _)| path.starts_with(prefix)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path.as_ref())) else { continue }; // binary
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            if lower.contains("openagc") || lower.contains("open agc") {
                found.push(format!("{path}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    if found.is_empty() {
        println!("no old name outside the allowed places");
        return Ok(());
    }
    for f in &found {
        eprintln!("{f}");
    }
    bail!("{} lines name OpenAGC; say Kaluta, or add the file to xtask/src/brand.rs with its reason", found.len())
}
