//! Snippets (spec §14.5 amendment 2026-11-18): saved text the user puts
//! into a message by name, in the global store beside global facts (ADR
//! 0012), so every account has them.

use mail_store::snippets as store;

use crate::{Core, CoreError, ErrorKind, runtime};

/// A snippet, as Swift sees it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnippetInfo {
    /// 0 until saved.
    pub id: i64,
    pub name: String,
    pub body_html: String,
    pub updated_at: i64,
}

impl From<store::Snippet> for SnippetInfo {
    fn from(s: store::Snippet) -> Self {
        SnippetInfo { id: s.id, name: s.name, body_html: s.body_html, updated_at: s.updated_at }
    }
}

impl From<SnippetInfo> for store::Snippet {
    fn from(s: SnippetInfo) -> Self {
        store::Snippet { id: s.id, name: s.name, body_html: s.body_html, updated_at: s.updated_at }
    }
}

fn invalid(e: mail_store::StoreError) -> CoreError {
    match e {
        mail_store::StoreError::Invalid(m) => CoreError::new(ErrorKind::InvalidInput, m),
        other => other.into(),
    }
}

#[uniffi::export]
impl Core {
    /// Every snippet, by name.
    pub async fn snippets(&self) -> Result<Vec<SnippetInfo>, CoreError> {
        let db = self.global_facts_db()?;
        runtime::run(async move { Ok(db.read(store::list).await?.into_iter().map(Into::into).collect()) }).await
    }

    /// Save a snippet (a new one when its id is 0); returns its id.
    pub async fn save_snippet(&self, snippet: SnippetInfo) -> Result<i64, CoreError> {
        let db = self.global_facts_db()?;
        let now = mail_sync::now_millis();
        let s: store::Snippet = snippet.into();
        runtime::run(async move { db.write(move |tx| store::save(tx, &s, now)).await.map_err(invalid) }).await
    }

    /// Delete a snippet; the snippet it was, for undo.
    pub async fn delete_snippet(&self, id: i64) -> Result<Option<SnippetInfo>, CoreError> {
        let db = self.global_facts_db()?;
        runtime::run(async move { Ok(db.write(move |tx| store::delete(tx, id)).await?.map(Into::into)) }).await
    }

    /// Put a deleted snippet back, with its id (undo).
    pub async fn restore_snippet(&self, snippet: SnippetInfo) -> Result<(), CoreError> {
        let db = self.global_facts_db()?;
        let s: store::Snippet = snippet.into();
        runtime::run(async move { Ok(db.write(move |tx| store::restore(tx, &s)).await?) }).await
    }
}
