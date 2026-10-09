//! Encryption at rest on the rules server, the app's side (spec §10.6,
//! oagc-gmn7.7; the primitives are `rules_crypto`).
//!
//! The app holds, per agent mailbox, an X25519 key pair (the private half
//! in the Keychain as `rules.report_key.<account>`) and the newest snapshot
//! key with its id (`rules.snapshot_key.<account>`). Each push seals the
//! snapshot under a fresh key and wraps that key for every live agent whose
//! key the server has sealed to the app; an agent that connects between
//! pushes gets a wrap of the newest key when the app next sees it in the
//! list. Reports come sealed to the app and are opened here.

use rules_crypto::{self as seal, AppKey, KeyWrap, SecretKey};

use super::*;

/// What one encrypted push seals with: the app's key pair, a fresh
/// snapshot key and id, and that key wrapped for the live agents.
pub(crate) struct Sealing {
    pub app: AppKey,
    pub key: SecretKey,
    pub key_id: String,
    pub wraps: Vec<KeyWrap>,
}

impl Sealing {
    /// The push body for `snapshot` (already versioned and hashed).
    pub fn body(&self, snapshot: &Snapshot) -> Result<String, Failure> {
        let json = zeroize::Zeroizing::new(snapshot.to_json().map_err(|e| Failure::Final(e.to_string()))?);
        let address = &snapshot.mailbox.address;
        let schema = snapshot.schema_version;
        let sealed = seal::seal_snapshot(&self.key, &self.key_id, address, snapshot.version, schema, &json);
        serde_json::to_string(&seal::SealedSnapshot {
            encryption: seal::ENCRYPTION_VERSION,
            key_id: self.key_id.clone(),
            version: snapshot.version,
            published_at: snapshot.published_at,
            schema_version: schema,
            address: address.clone(),
            ciphertext: seal::b64(&sealed),
            app_key: self.app.public_base64(),
            wraps: self.wraps.clone(),
        })
        .map_err(|e| Failure::Final(e.to_string()))
    }
}

/// The snapshot key as kept in the Keychain.
#[derive(Serialize, Deserialize)]
struct StoredKey {
    key_id: String,
    key: String,
}

/// Wraps of `key` for each live agent whose key the server sealed to this
/// app (with `only_unreadable`, those that cannot read the newest version).
pub(crate) fn wraps_for(
    app: &AppKey,
    agents: &Value,
    key: &SecretKey,
    key_id: &str,
    only_unreadable: bool,
) -> Vec<KeyWrap> {
    let Some(list) = agents["agent_tokens"].as_array() else { return vec![] };
    list.iter()
        .filter(|a| a["revoked_at"].is_null() && !(only_unreadable && a["readable"] == true))
        .filter_map(|a| {
            let id = a["id"].as_str()?;
            let sealed = seal::unb64(a["agent_key"].as_str()?).ok()?;
            match seal::open_agent_key_for_app(app, &sealed, id) {
                Ok(agent) => Some(KeyWrap {
                    agent_id: id.to_owned(),
                    wrap: seal::b64(&seal::wrap_snapshot_key(&agent, key, id, key_id)),
                }),
                // Sealed to a key this Mac no longer has: the server seals it
                // again to the new one at the agent's next request.
                Err(_) => None,
            }
        })
        .collect()
}

/// A report from the server's list with its sealed fields opened, or as it
/// is when it was not sealed. `None` when it was sealed and does not open
/// (sealed to a key this Mac has lost).
pub(crate) fn opened_report(app: Option<&AppKey>, address: &str, mut v: Value) -> Option<Value> {
    let Some(sealed) = v["sealed"].as_str().map(str::to_owned) else { return Some(v) };
    let agent_id = v["agent_id"].as_str().unwrap_or_default().to_owned();
    let bytes = seal::unb64(&sealed).ok()?;
    let opened = seal::open_for_app(app?, &bytes, &seal::report_context(address, &agent_id)).ok()?;
    let fields: Value = serde_json::from_slice(&opened).ok()?;
    for k in ["message_id", "to", "subject", "body_markdown"] {
        v[k] = fields[k].clone();
    }
    v["check"]["guide_check"] = fields["guide_check"].clone();
    Some(v)
}

impl Core {
    /// The mailbox's key pair, made and kept in the Keychain the first time
    /// when `create`.
    pub(crate) fn rules_app_key(&self, account_id: &str, create: bool) -> Result<Option<AppKey>, CoreError> {
        let name = keys::rules_report_key(account_id);
        if let Some(stored) = self.secrets.get(name.clone())?
            && let Ok(key) = AppKey::from_base64(&stored)
        {
            return Ok(Some(key));
        }
        if !create {
            return Ok(None);
        }
        let key = AppKey::generate();
        self.secrets.set(name, key.to_base64().to_string())?;
        Ok(Some(key))
    }

    /// The newest snapshot key and its id, as the last push kept them.
    pub(crate) fn rules_snapshot_key(&self, account_id: &str) -> Result<Option<(SecretKey, String)>, CoreError> {
        let Some(stored) = self.secrets.get(keys::rules_snapshot_key(account_id))? else { return Ok(None) };
        let stored = zeroize::Zeroizing::new(stored);
        let Ok(s) = serde_json::from_str::<StoredKey>(&stored) else { return Ok(None) };
        let key = zeroize::Zeroizing::new(s.key);
        Ok(SecretKey::from_base64(&key).ok().map(|k| (k, s.key_id)))
    }

    fn set_rules_snapshot_key(&self, account_id: &str, key: &SecretKey, key_id: &str) -> Result<(), CoreError> {
        let json = zeroize::Zeroizing::new(
            serde_json::to_string(&StoredKey { key_id: key_id.to_owned(), key: key.to_base64().to_string() })
                .map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?,
        );
        self.secrets.set(keys::rules_snapshot_key(account_id), json.to_string())
    }

    /// Forget the mailbox's keys (it stopped publishing and left the server).
    pub(crate) fn rules_forget_keys(&self, account_id: &str) {
        let _ = self.secrets.delete(keys::rules_snapshot_key(account_id));
        let _ = self.secrets.delete(keys::rules_report_key(account_id));
    }

    /// The mailbox's agents on its server, as the publisher lists them.
    pub(crate) async fn rules_list_agents(
        &self,
        server: &Server,
        address: &str,
        token: &str,
    ) -> Result<Value, Failure> {
        let request =
            client(&self.rules)?.get(server.endpoint(&["v1", "mailboxes", address, "agent-tokens"])).bearer_auth(token);
        let a = send(request, &server.host).await?;
        if let Some(f) = transient(&server.host, &a) {
            return Err(f);
        }
        match a.status {
            200 => Ok(a.body),
            // Forgotten by the server (401 as for a wrong token; 404 from
            // older servers): the push registers again, or says the token
            // is refused.
            401 | 404 => Ok(json!({ "agent_tokens": [] })),
            _ => Err(Failure::Final(format!("{} did not list the agents: {}", server.host, a.says()))),
        }
    }

    /// Everything one encrypted push needs: the key pair (made the first
    /// time), a fresh snapshot key kept in the Keychain, and its wraps for
    /// the live agents.
    pub(crate) async fn rules_sealing(
        &self,
        account_id: &str,
        server: &Server,
        address: &str,
        token: &str,
    ) -> Result<Sealing, Failure> {
        let app = self
            .rules_app_key(account_id, true)?
            .ok_or_else(|| Failure::Final("could not make the mailbox's key".into()))?;
        let agents = self.rules_list_agents(server, address, token).await?;
        let key = SecretKey::random();
        let key_id = seal::new_key_id();
        let wraps = wraps_for(&app, &agents, &key, &key_id, false);
        // Kept before the push, so an agent connecting meanwhile can be given
        // it; a push that fails leaves an id the server does not have, and
        // nothing is wrapped with it.
        self.set_rules_snapshot_key(account_id, &key, &key_id)?;
        Ok(Sealing { app, key, key_id, wraps })
    }

    /// Wrap the newest snapshot key for agents that connected since it was
    /// pushed (spec §10.6). How many were given it.
    pub(crate) async fn rules_rewrap(&self, account_id: &str) -> Result<usize, Failure> {
        let Some(record) = self.rules_record(account_id).filter(|r| r.enabled && !r.sample) else { return Ok(0) };
        let Some(key_id) = record.key_id.clone() else { return Ok(0) };
        let Some((key, kept_id)) = self.rules_snapshot_key(account_id)? else { return Ok(0) };
        let Some(app) = self.rules_app_key(account_id, false)? else { return Ok(0) };
        if kept_id != key_id {
            return Ok(0);
        }
        let server = parse_server(&record.server_url)?;
        let address = self.agent_meta_or_err(account_id)?.address;
        let Some(token) = self.secrets.get(keys::rules_publish_token(&server.key, account_id))? else { return Ok(0) };
        let agents = self.rules_list_agents(&server, &address, &token).await?;
        // The server's newest is another (a push in flight): the next push
        // wraps for everyone.
        if agents["key_id"].as_str() != Some(key_id.as_str()) {
            return Ok(0);
        }
        let wraps = wraps_for(&app, &agents, &key, &key_id, true);
        if wraps.is_empty() {
            return Ok(0);
        }
        let request = client(&self.rules)?
            .post(server.endpoint(&["v1", "mailboxes", &address, "snapshot", "keys"]))
            .bearer_auth(&token)
            .json(&json!({ "key_id": key_id, "wraps": wraps }));
        let a = send(request, &server.host).await?;
        if let Some(f) = transient(&server.host, &a) {
            return Err(f);
        }
        match a.status {
            200 => {
                let stored = usize::try_from(a.body["stored"].as_u64().unwrap_or(0)).unwrap_or(0);
                tracing::info!(account = account_id, stored, "snapshot key wrapped for new agents");
                Ok(stored)
            }
            // A push replaced that key meanwhile.
            404 => Ok(0),
            _ => Err(Failure::Final(format!("{} did not take the keys: {}", server.host, a.says()))),
        }
    }

    /// Wrap for new agents in the background, if any wait for it.
    pub(crate) fn rules_rewrap_soon(&self, account_id: &str, agents: &Value) {
        let waiting = agents["agent_tokens"].as_array().is_some_and(|list| {
            list.iter().any(|a| a["revoked_at"].is_null() && a["agent_key"].is_string() && a["readable"] == false)
        });
        let Some(core) = self.me.upgrade() else { return };
        if !waiting {
            return;
        }
        let account = account_id.to_owned();
        runtime::runtime().spawn(async move {
            if let Err(f) = core.rules_rewrap(&account).await {
                tracing::warn!(
                    account = account.as_str(),
                    error = f.message(),
                    "snapshot key not wrapped for new agents"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_open_with_the_apps_key_and_plain_ones_pass_through() {
        let app = AppKey::generate();
        let public = seal::app_public_key(&app.public_base64()).unwrap();
        let fields = json!({ "message_id": "m@x", "to": ["ann@acme.com"], "subject": "Plan",
                             "body_markdown": "Hi", "guide_check": ["x"] });
        let sealed = seal::seal_for_app(
            &public,
            fields.to_string().as_bytes(),
            &seal::report_context("scout@agents.example", "0123456789abcdef"),
        )
        .unwrap();
        let listed = json!({ "id": 1, "agent_id": "0123456789abcdef", "subject": "", "to": [],
                             "check": { "version": 2, "guide_check": [] }, "sealed": seal::b64(&sealed) });
        let opened = opened_report(Some(&app), "scout@agents.example", listed.clone()).unwrap();
        assert_eq!(opened["subject"], "Plan");
        assert_eq!(opened["to"], json!(["ann@acme.com"]));
        assert_eq!(opened["check"], json!({ "version": 2, "guide_check": ["x"] }));
        assert!(opened_report(Some(&AppKey::generate()), "scout@agents.example", listed.clone()).is_none());
        assert!(opened_report(None, "scout@agents.example", listed).is_none());
        let plain = json!({ "id": 2, "subject": "Plain" });
        assert_eq!(opened_report(None, "scout@agents.example", plain.clone()), Some(plain));
    }
}
