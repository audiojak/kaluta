//! *Connect an Agent…* (spec §10.1, mailbox mode): the MCP entry that lets
//! Claude Code or Codex use one agent mailbox through the app's bundled
//! `kaluta-mcp --mailbox <address>`. The app shows exactly what will be
//! written and where before writing it, backs the file up first, and
//! always offers the command to paste instead.
//!
//! - Claude Code: the user-scope entry `claude mcp add --scope user` makes,
//!   under `mcpServers` in `~/.claude.json`.
//! - Codex: a `[mcp_servers.<name>]` table in `~/.codex/config.toml`
//!   (`$CODEX_HOME/config.toml` when that is set).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{Core, CoreError, ErrorKind};

/// An agent CLI that can be connected to a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentClient {
    ClaudeCode,
    Codex,
}

/// What *Connect an Agent…* would write, shown before it does.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentConnection {
    pub client: AgentClient,
    /// The MCP server's name in the agent's config: `kaluta-writer`.
    pub server_name: String,
    /// The file written.
    pub config_path: String,
    /// Exactly what goes into it: the JSON entry or the TOML table.
    pub entry: String,
    /// The file exists, and is backed up before it is changed.
    pub file_exists: bool,
    /// The file already has an entry of this name, which is replaced.
    pub replaces: bool,
    /// The entry the app wrote for this mailbox before it was named Kaluta
    /// (`openagc-writer`), if the file has one: it goes, replaced by ours.
    pub replaces_old_name: Option<String>,
    /// The command to run instead of letting Kaluta write the file.
    pub paste: String,
}

/// Codex waits this long for a tool: a send may wait for the user's
/// approval (10 minutes, spec §10.4), as for the app's own sessions.
const CODEX_TOOL_TIMEOUT_SEC: u32 = 900;

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::InvalidInput, message.into())
}

fn storage(e: std::io::Error) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

/// `kaluta-writer` for `writer@jade-emu.primitive.email`.
pub(crate) fn server_name(address: &str) -> String {
    let local = address.split('@').next().unwrap_or(address);
    format!("kaluta-{}", crate::agent_mailbox::local_part(local))
}

/// The name [`server_name`] gave before the project was named Kaluta.
fn old_server_name(address: &str) -> String {
    let local = address.split('@').next().unwrap_or(address);
    format!("openagc-{}", crate::agent_mailbox::local_part(local))
}

/// A word for a POSIX shell, quoted when it needs it.
fn shell_word(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c)) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The config file for `client` under `home`.
fn config_path(home: &Path, client: AgentClient, codex_home: Option<&Path>) -> PathBuf {
    match client {
        AgentClient::ClaudeCode => home.join(".claude.json"),
        AgentClient::Codex => codex_home.map_or_else(|| home.join(".codex"), Path::to_path_buf).join("config.toml"),
    }
}

/// What connecting would write, from the files as they are now.
pub(crate) fn plan(
    home: &Path,
    codex_home: Option<&Path>,
    client: AgentClient,
    shim: &Path,
    address: &str,
) -> Result<AgentConnection, CoreError> {
    let name = server_name(address);
    let old_name = old_server_name(address);
    let path = config_path(home, client, codex_home);
    let shim_text = shim.to_string_lossy().into_owned();
    let existing = std::fs::read_to_string(&path).ok();
    let (entry, replaces, has_old, paste) = match client {
        AgentClient::ClaudeCode => {
            let entry = claude_entry(&shim_text, address);
            let parsed = existing.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok());
            let has = |n: &str| parsed.as_ref().is_some_and(|v| v["mcpServers"].get(n).is_some());
            let (replaces, has_old) = (has(&name), has(&old_name));
            let body = serde_json::to_string_pretty(&json!({ &name: entry })).unwrap_or_default();
            let paste = format!(
                "claude mcp add --scope user {name} -- {} --mailbox {}",
                shell_word(&shim_text),
                shell_word(address)
            );
            (body, replaces, has_old, paste)
        }
        AgentClient::Codex => {
            let table = codex_table(&name, &shim_text, address);
            let parsed = existing.as_deref().and_then(|t| t.parse::<toml::Table>().ok());
            let has = |n: &str| parsed.as_ref().is_some_and(|t| t.get("mcp_servers").and_then(|s| s.get(n)).is_some());
            let (replaces, has_old) = (has(&name), has(&old_name));
            let paste = format!("codex mcp add {name} -- {} --mailbox {}", shell_word(&shim_text), shell_word(address));
            (table, replaces, has_old, paste)
        }
    };
    Ok(AgentConnection {
        client,
        server_name: name,
        config_path: path.to_string_lossy().into_owned(),
        entry,
        file_exists: existing.is_some(),
        replaces,
        replaces_old_name: has_old.then_some(old_name),
        paste,
    })
}

fn claude_entry(shim: &str, address: &str) -> Value {
    json!({ "type": "stdio", "command": shim, "args": ["--mailbox", address], "env": {} })
}

fn codex_table(name: &str, shim: &str, address: &str) -> String {
    format!(
        "[mcp_servers.{name}]\ncommand = {}\nargs = [\"--mailbox\", {}]\ntool_timeout_sec = {CODEX_TOOL_TIMEOUT_SEC}\n",
        toml_string(shim),
        toml_string(address)
    )
}

/// The file with the entry in it: the rest of the file as it was (Claude
/// Code's JSON is rewritten, its content unchanged).
fn updated(
    existing: Option<&str>,
    connection: &AgentConnection,
    shim: &str,
    address: &str,
) -> Result<String, CoreError> {
    let name = &connection.server_name;
    match connection.client {
        AgentClient::ClaudeCode => {
            let mut root = match existing.map(str::trim).filter(|t| !t.is_empty()) {
                Some(text) => serde_json::from_str::<Value>(text).map_err(|e| {
                    invalid(format!("{} is not valid JSON ({e}); paste the command instead", connection.config_path))
                })?,
                None => json!({}),
            };
            let Some(object) = root.as_object_mut() else {
                return Err(invalid(format!(
                    "{} is not a JSON object; paste the command instead",
                    connection.config_path
                )));
            };
            let servers = object.entry("mcpServers").or_insert_with(|| json!({}));
            let Some(servers) = servers.as_object_mut() else {
                return Err(invalid("its mcpServers is not an object; paste the command instead"));
            };
            if let Some(old) = &connection.replaces_old_name {
                servers.remove(old);
            }
            servers.insert(name.clone(), claude_entry(shim, address));
            let mut text = serde_json::to_string_pretty(&root).map_err(|e| invalid(e.to_string()))?;
            text.push('\n');
            Ok(text)
        }
        AgentClient::Codex => {
            let original = existing.unwrap_or_default();
            if original.parse::<toml::Table>().is_err() {
                return Err(invalid(format!(
                    "{} is not valid TOML; paste the command instead",
                    connection.config_path
                )));
            }
            // Drop an earlier table of this name or the old one (and their
            // sub-tables), then append ours.
            let mut headers = vec![format!("[mcp_servers.{name}"), format!("[mcp_servers.\"{name}\"")];
            if let Some(old) = &connection.replaces_old_name {
                headers.extend([format!("[mcp_servers.{old}"), format!("[mcp_servers.\"{old}\"")]);
            }
            let mut kept = Vec::new();
            let mut skipping = false;
            for line in original.lines() {
                let t = line.trim_start();
                if t.starts_with('[') {
                    skipping = headers.iter().any(|h| {
                        t.strip_prefix(h.as_str()).is_some_and(|rest| rest.starts_with(']') || rest.starts_with('.'))
                    });
                }
                if !skipping {
                    kept.push(line);
                }
            }
            let mut text = kept.join("\n");
            while text.ends_with('\n') {
                text.pop();
            }
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&connection.entry);
            // The result must read back as the entry, or nothing is written
            // (the file may define mcp_servers in a form this cannot edit).
            let parsed = text.parse::<toml::Table>().map_err(|e| {
                invalid(format!(
                    "could not add the entry to {} safely ({e}); paste the command instead",
                    connection.config_path
                ))
            })?;
            let command = parsed.get("mcp_servers").and_then(|s| s.get(name)).and_then(|s| s.get("command"));
            if command.and_then(|c| c.as_str()) != Some(shim) {
                return Err(invalid(format!(
                    "could not add the entry to {} safely; paste the command instead",
                    connection.config_path
                )));
            }
            Ok(text)
        }
    }
}

/// Write the entry: the file is copied to `<file>.kaluta-backup-<time>`
/// first, then replaced atomically with its permissions kept. Returns the
/// backup's path, if there was a file to back up.
pub(crate) fn write(
    home: &Path,
    codex_home: Option<&Path>,
    client: AgentClient,
    shim: &Path,
    address: &str,
) -> Result<Option<String>, CoreError> {
    let connection = plan(home, codex_home, client, shim, address)?;
    let path = PathBuf::from(&connection.config_path);
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(storage(e)),
    };
    let text = updated(existing.as_deref(), &connection, &shim.to_string_lossy(), address)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(storage)?;
    }
    let backup = match &existing {
        Some(_) => {
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let backup = path.with_file_name(format!("{file}.kaluta-backup-{stamp}"));
            std::fs::copy(&path, &backup).map_err(storage)?;
            Some(backup.to_string_lossy().into_owned())
        }
        None => None,
    };
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
    replace(&path, text.as_bytes(), mode, |_| Ok(())).map_err(storage)?;
    Ok(backup)
}

/// Replace `path` with `contents` atomically: a temp file beside it,
/// created new (never following a planted link) with `mode` from the
/// start, so the config's contents (which may hold other servers' tokens)
/// are never readable by others even for a moment; then renamed over it.
/// The temp file is removed on any error. `before_rename` lets tests look
/// at the temp file and force a failure.
fn replace(
    path: &Path,
    contents: &[u8],
    mode: u32,
    before_rename: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos());
    let tmp = path.with_file_name(format!(".{file}.kaluta-tmp-{}-{}", std::process::id(), nanos.unwrap_or(0)));
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp)?;
    let result = (|| {
        // The umask may have taken bits from `mode`; never adds any.
        out.set_permissions(std::fs::Permissions::from_mode(mode))?;
        out.write_all(contents)?;
        out.sync_all()?;
        drop(out);
        before_rename(&tmp)?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

impl Core {
    fn connect_inputs(&self, account_id: &str) -> Result<(PathBuf, Option<PathBuf>, PathBuf, String), CoreError> {
        let meta = self.agent_meta_or_err(account_id)?;
        let shim = self.agents.resources.read().unwrap_or_else(|e| e.into_inner()).shim_path.clone();
        if shim.as_os_str().is_empty() {
            return Err(CoreError::new(ErrorKind::NotFound, "Kaluta does not know where kaluta-mcp is"));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no home directory"))?;
        let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
        Ok((home, codex_home, shim, meta.address))
    }
}

#[uniffi::export]
impl Core {
    /// What *Connect an Agent…* would write for this agent mailbox, and
    /// where (spec §10.1). Writes nothing.
    pub fn agent_connection(&self, account_id: String, client: AgentClient) -> Result<AgentConnection, CoreError> {
        let (home, codex_home, shim, address) = self.connect_inputs(&account_id)?;
        plan(&home, codex_home.as_deref(), client, &shim, &address)
    }

    /// Write it: the config file is backed up first. Returns the backup's
    /// path, if there was a file.
    pub fn connect_agent(&self, account_id: String, client: AgentClient) -> Result<Option<String>, CoreError> {
        let (home, codex_home, shim, address) = self.connect_inputs(&account_id)?;
        write(&home, codex_home.as_deref(), client, &shim, &address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A scratch HOME: tests never touch the real ~/.claude.json or ~/.codex.
    fn home(name: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("kaluta-connect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }

    const SHIM: &str = "/Applications/Kaluta.app/Contents/MacOS/kaluta-mcp";
    const ADDRESS: &str = "writer@jade-emu.primitive.email";

    fn backups(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".kaluta-backup-"))
            .collect()
    }

    #[test]
    fn claude_codes_user_entry_is_shown_then_written_beside_what_was_there() {
        let h = home("claude");
        let shim = Path::new(SHIM);
        let file = h.0.join(".claude.json");
        let plan = plan(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap();
        assert_eq!(plan.server_name, "kaluta-writer");
        assert_eq!(plan.config_path, file.to_string_lossy());
        assert!(!plan.file_exists && !plan.replaces);
        let shown: Value = serde_json::from_str(&plan.entry).unwrap();
        assert_eq!(shown["kaluta-writer"]["args"], json!(["--mailbox", ADDRESS]));
        assert_eq!(plan.paste, format!("claude mcp add --scope user kaluta-writer -- {SHIM} --mailbox {ADDRESS}"));

        // A file with the user's own settings and servers.
        std::fs::write(&file, r#"{"numStartups": 7, "mcpServers": {"github": {"command": "gh-mcp"}}}"#).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let backup = write(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            r#"{"numStartups": 7, "mcpServers": {"github": {"command": "gh-mcp"}}}"#,
            "backed up as it was"
        );
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after["numStartups"], 7);
        assert_eq!(after["mcpServers"]["github"]["command"], "gh-mcp");
        assert_eq!(after["mcpServers"]["kaluta-writer"], shown["kaluta-writer"]);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "permissions kept");

        // Connecting again replaces the entry; it never duplicates it.
        assert!(super::plan(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap().replaces);
        std::thread::sleep(std::time::Duration::from_millis(1100)); // a backup per second
        write(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap();
        let again: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(again["mcpServers"].as_object().unwrap().len(), 2);
        assert_eq!(backups(&h.0).len(), 2);

        // A file that is not JSON is left alone.
        std::fs::write(&file, "not json").unwrap();
        assert!(write(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
    }

    #[test]
    fn codexs_table_is_appended_or_replaced_and_must_read_back() {
        let h = home("codex");
        let shim = Path::new("/Applications/Kaluta Beta.app/Contents/MacOS/kaluta-mcp");
        let file = h.0.join(".codex/config.toml");
        let plan = plan(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap();
        assert_eq!(plan.config_path, file.to_string_lossy());
        assert_eq!(
            plan.entry,
            "[mcp_servers.kaluta-writer]\ncommand = \"/Applications/Kaluta Beta.app/Contents/MacOS/kaluta-mcp\"\n\
             args = [\"--mailbox\", \"writer@jade-emu.primitive.email\"]\ntool_timeout_sec = 900\n"
        );
        assert_eq!(
            plan.paste,
            "codex mcp add kaluta-writer -- '/Applications/Kaluta Beta.app/Contents/MacOS/kaluta-mcp' --mailbox \
             writer@jade-emu.primitive.email"
        );

        // No file yet: it is created, with nothing to back up.
        assert_eq!(write(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), plan.entry);

        // The user's settings and an older entry of the same name.
        let before = "model = \"o4\"\n\n[mcp_servers.kaluta-writer]\ncommand = \"/old/kaluta-mcp\"\n\n\
                      [mcp_servers.kaluta-writer.env]\nX = \"1\"\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\n";
        std::fs::write(&file, before).unwrap();
        assert!(super::plan(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap().replaces);
        let backup = write(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), before);
        let after = std::fs::read_to_string(&file).unwrap();
        let table: toml::Table = after.parse().unwrap();
        assert_eq!(table["model"].as_str(), Some("o4"));
        assert_eq!(table["mcp_servers"]["github"]["command"].as_str(), Some("gh-mcp"));
        let ours = &table["mcp_servers"]["kaluta-writer"];
        assert_eq!(ours["command"].as_str(), Some(shim.to_str().unwrap()));
        assert!(ours.get("env").is_none(), "the old entry's sub-table went with it");
        assert_eq!(after.matches("[mcp_servers.kaluta-writer]").count(), 1);

        // CODEX_HOME moves the file.
        let elsewhere = h.0.join("codex-home");
        let moved = super::plan(&h.0, Some(&elsewhere), AgentClient::Codex, shim, ADDRESS).unwrap();
        assert_eq!(moved.config_path, elsewhere.join("config.toml").to_string_lossy());

        // A form this cannot edit safely is refused, the file untouched.
        let inline = "mcp_servers = { github = { command = \"gh\" } }\n";
        std::fs::write(&file, inline).unwrap();
        assert!(write(&h.0, None, AgentClient::Codex, shim, ADDRESS).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), inline);
    }

    #[test]
    fn an_entry_written_before_the_rename_is_replaced_by_ours() {
        let h = home("old-name");
        let shim = Path::new(SHIM);
        let claude = h.0.join(".claude.json");
        std::fs::write(
            &claude,
            r#"{"mcpServers": {"openagc-writer": {"command": "/Applications/OpenAGC.app/Contents/MacOS/openagc-mcp"},
                "openagc-scout": {"command": "/x/openagc-mcp"}}}"#,
        )
        .unwrap();
        let shown = plan(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap();
        assert_eq!(shown.replaces_old_name.as_deref(), Some("openagc-writer"));
        assert!(!shown.replaces);
        write(&h.0, None, AgentClient::ClaudeCode, shim, ADDRESS).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&claude).unwrap()).unwrap();
        assert!(after["mcpServers"].get("openagc-writer").is_none());
        assert_eq!(after["mcpServers"]["kaluta-writer"]["command"], SHIM);
        assert!(after["mcpServers"].get("openagc-scout").is_some(), "another mailbox's entry is its own to update");

        let codex = h.0.join(".codex/config.toml");
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(
            &codex,
            "[mcp_servers.openagc-writer]\ncommand = \"/x/openagc-mcp\"\n\n[mcp_servers.openagc-writer.env]\nX = \"1\"\n",
        )
        .unwrap();
        assert_eq!(
            plan(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap().replaces_old_name.as_deref(),
            Some("openagc-writer")
        );
        write(&h.0, None, AgentClient::Codex, shim, ADDRESS).unwrap();
        let table: toml::Table = std::fs::read_to_string(&codex).unwrap().parse().unwrap();
        assert!(table["mcp_servers"].get("openagc-writer").is_none());
        assert_eq!(table["mcp_servers"]["kaluta-writer"]["command"].as_str(), Some(SHIM));
    }

    fn leftovers(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains("kaluta-tmp"))
            .collect()
    }

    #[test]
    fn the_temp_copy_is_private_from_creation_and_never_left_behind() {
        use std::os::unix::fs::PermissionsExt;
        let h = home("temp");
        let file = h.0.join(".claude.json");
        std::fs::write(&file, r#"{"mcpServers": {"github": {"env": {"TOKEN": "secret"}}}}"#).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        // The process umask (022 here) would make a plain write 0644.
        let mut seen = None;
        let failed = replace(&file, b"{}", 0o600, |tmp| {
            seen = Some(std::fs::symlink_metadata(tmp).unwrap().permissions().mode() & 0o777);
            Err(std::io::Error::other("forced"))
        });
        assert!(failed.is_err());
        assert_eq!(seen, Some(0o600), "no group or other bits, ever");
        assert!(leftovers(&h.0).is_empty(), "the temp file is removed on failure");
        assert!(std::fs::read_to_string(&file).unwrap().contains("secret"), "the file is untouched");

        // A replace that succeeds keeps the mode and leaves nothing behind.
        replace(&file, b"{\"a\": 1}", 0o600, |_| Ok(())).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"a\": 1}");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(leftovers(&h.0).is_empty());

        // The whole write: the temp file is 0600 for a new file too.
        let codex = h.0.join(".codex/config.toml");
        write(&h.0, None, AgentClient::Codex, Path::new(SHIM), ADDRESS).unwrap();
        assert_eq!(std::fs::metadata(&codex).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(leftovers(&h.0.join(".codex")).is_empty());
    }
}
