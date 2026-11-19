//! Reply-landed alerts (spec §14.7 amendment 2026-11-18): the user asks to
//! be told as soon as someone answers a conversation; the app tells them
//! once, even with Kaluta in front.

use mail_domain::ThreadId;
use mail_store::watches as store;

use crate::{Core, CoreError, runtime};

#[uniffi::export]
impl Core {
    /// Watch a conversation for an answer, or (`false`) no longer;
    /// whether it was watched, for undo. Thread ids cross as text, as
    /// everywhere at the Swift bridge.
    pub async fn watch_thread(&self, thread_id: String, on: bool) -> Result<bool, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let thread = ThreadId(thread_id);
        runtime::run(async move { Ok(db.write(move |tx| store::set(tx, &thread, on, now)).await?) }).await
    }

    /// Every watched conversation.
    pub async fn watched_threads(&self) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::list).await?.into_iter().map(|t| t.0).collect()) }).await
    }

    /// Of the conversations mail just arrived in, the watched ones someone
    /// else answered: their watches end, so each is told once.
    pub async fn replies_landed(&self, thread_ids: Vec<String>) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        let threads: Vec<ThreadId> = thread_ids.into_iter().map(ThreadId).collect();
        runtime::run(async move {
            Ok(db.write(move |tx| store::landed(tx, &threads)).await?.into_iter().map(|t| t.0).collect())
        })
        .await
    }
}
