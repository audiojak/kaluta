//! Search by meaning (spec §14.3 amendment 2026-11-18, ADR 0023). The app
//! computes vectors on the Mac with Apple's on-device sentence model and
//! hands them here; the core keeps them and finds the nearest
//! conversations. Nothing leaves the Mac. Ids cross the Swift bridge as
//! text, as everywhere there.

use mail_domain::MessageId;
use mail_store::embeddings as store;

use crate::ffi::ThreadRow;
use crate::{Core, CoreError, ErrorKind, runtime};

/// Matches less alike than this are not offered.
const MIN_SCORE: f32 = 0.55;

/// A message for the app to embed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EmbeddingInput {
    pub message_id: String,
    pub text: String,
}

/// A message's vector, from the app.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EmbeddingVector {
    pub message_id: String,
    pub vector: Vec<f32>,
}

/// How far the index has got, for the sync footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct EmbeddingProgress {
    pub done: u32,
    pub total: u32,
}

#[uniffi::export]
impl Core {
    /// Messages with no vector from `model` yet, newest first.
    pub async fn embedding_backlog(&self, model: String, limit: u32) -> Result<Vec<EmbeddingInput>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| store::pending(c, &model, limit.min(500)))
                .await?
                .into_iter()
                .map(|t| EmbeddingInput { message_id: t.message_id.0, text: t.text })
                .collect())
        })
        .await
    }

    /// Keep the app's vectors; how many were kept.
    pub async fn store_embeddings(&self, model: String, vectors: Vec<EmbeddingVector>) -> Result<u32, CoreError> {
        let db = self.db()?;
        let rows: Vec<(MessageId, Vec<f32>)> =
            vectors.into_iter().map(|v| (MessageId(v.message_id), v.vector)).collect();
        runtime::run(async move {
            db.write(move |tx| store::store(tx, &model, &rows)).await.map(|n| n as u32).map_err(|e| match e {
                mail_store::StoreError::Invalid(m) => CoreError::new(ErrorKind::InvalidInput, m),
                other => other.into(),
            })
        })
        .await
    }

    pub async fn embedding_progress(&self, model: String) -> Result<EmbeddingProgress, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let (done, total) = db.read(move |c| store::progress(c, &model)).await?;
            Ok(EmbeddingProgress { done, total })
        })
        .await
    }

    /// The conversations nearest a query's vector, best first.
    pub async fn search_by_meaning(
        &self,
        model: String,
        vector: Vec<f32>,
        limit: u32,
    ) -> Result<Vec<ThreadRow>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let found = store::nearest(c, &model, &vector, MIN_SCORE, limit as usize)?;
                    let mut rows = Vec::new();
                    for (thread, _) in found {
                        if let Some(summary) = mail_store::read::get_thread_summary(c, &thread)? {
                            rows.push(ThreadRow::from(summary));
                        }
                    }
                    Ok(rows)
                })
                .await?)
        })
        .await
    }
}
