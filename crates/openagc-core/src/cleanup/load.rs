//! Clean Up loads every header (spec §14.12): it needs the whole mailbox,
//! so opening it widens a Gmail account's sync window to *Everything*.
//! With IMAP the older mail comes down as headers only and the body window
//! keeps bodies where they were; without it, headers cost as much as whole
//! messages, so the window states the count and the time and asks first.
//! If IMAP is refused part way, the headers Clean Up asked for wait for it
//! and the window asks the same question before loading the rest whole.

use crate::account::SyncWindow;
use crate::registry::AccountKind;
use crate::{Core, CoreError, ErrorKind, runtime};

/// Where an account's download stands, for Clean Up's decision and its
/// progress line.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupLoadStatus {
    /// A Gmail account: the only kind with a sync window. Imported
    /// mailboxes hold everything they have; agent mailboxes list all.
    pub has_sync_window: bool,
    pub window: SyncWindow,
    /// Headers come cheaply (IMAP granted and not refused just now), so
    /// loading every header needs no asking.
    pub cheap_headers: bool,
    /// Messages waiting for headers only: older mail being loaded.
    pub headers_waiting: u64,
    /// Messages waiting for a whole download.
    pub bodies_waiting: u64,
    /// Clean Up's header load waits for IMAP (refused since it began):
    /// the rest would come down whole over the API, so the window asks
    /// first (`cleanup_load_waiting_headers`). Accounts whose window was
    /// *Everything* before Clean Up download it whole without asking.
    pub headers_paused: bool,
}

/// What loading all mail without IMAP would take.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupLoadEstimate {
    /// Messages in Gmail not stored here (Gmail's count less the stored
    /// one); none when Gmail could not be asked (offline, not syncing).
    pub messages: Option<u64>,
    /// About how long downloading them over the API takes, at the rate
    /// the backfill runs at.
    pub seconds: Option<u64>,
}

#[uniffi::export]
impl Core {
    /// Where `account_id`'s download stands (spec §14.12).
    pub async fn cleanup_load_status(&self, account_id: String) -> Result<CleanupLoadStatus, CoreError> {
        let entry = crate::registry::load_index(&self.data_path()).into_iter().find(|e| e.id == account_id);
        let Some(entry) = entry.filter(|e| e.kind == AccountKind::Gmail) else {
            return Ok(CleanupLoadStatus {
                has_sync_window: false,
                window: SyncWindow::Everything,
                cheap_headers: false,
                headers_waiting: 0,
                bodies_waiting: 0,
                headers_paused: false,
            });
        };
        let db = self.store_for(&account_id).await?;
        let service = self.accounts_sync_service(&account_id);
        // Not syncing yet: IMAP is what it will use if granted.
        let cheap_headers = entry.imap == Some(true) && service.as_ref().is_none_or(|s| s.engine().cheap_headers());
        runtime::run(async move {
            let (window, (bodies_waiting, headers_waiting)) = db
                .read(|c| {
                    let window = mail_store::read::sync_state(c, mail_sync::KEY_WINDOW)?;
                    Ok((window, mail_store::queue::counts(c)?))
                })
                .await?;
            let window = window.as_deref().and_then(mail_sync::SyncWindow::parse).unwrap_or_default().into();
            let headers_paused = mail_sync::cleanup_headers_paused(&db, cheap_headers).await?;
            Ok(CleanupLoadStatus {
                has_sync_window: true,
                window,
                cheap_headers,
                headers_waiting,
                bodies_waiting,
                headers_paused,
            })
        })
        .await
    }

    /// Set `account_id`'s sync window to *Everything*, keeping bodies
    /// where they were (`BodyWindow::kept_from`): with IMAP the older mail
    /// is queued for headers only. Returns false when the window was
    /// *Everything* already.
    pub async fn cleanup_load_every_header(&self, account_id: String) -> Result<bool, CoreError> {
        let entry = crate::registry::load_index(&self.data_path())
            .into_iter()
            .find(|e| e.id == account_id && e.kind == AccountKind::Gmail);
        let Some(entry) = entry else {
            return Err(CoreError::new(ErrorKind::InvalidInput, "only a Gmail account has a sync window"));
        };
        // Not syncing: it will use IMAP if granted.
        let imap = entry.imap == Some(true);
        let db = self.store_for(&account_id).await?;
        let service = self.accounts_sync_service(&account_id);
        runtime::run(async move {
            match service {
                Some(service) => {
                    let widened = service.engine().load_every_header().await?;
                    service.sync_now();
                    Ok(widened)
                }
                None => Ok(mail_sync::load_every_header_stored(&db, imap).await?),
            }
        })
        .await
    }

    /// *Load All Mail* while Clean Up's header load waits for IMAP
    /// (`CleanupLoadStatus::headers_paused`): the messages still waiting
    /// for headers come down whole over the API instead. Returns how many
    /// were queued.
    pub async fn cleanup_load_waiting_headers(&self, account_id: String) -> Result<u64, CoreError> {
        let db = self.store_for(&account_id).await?;
        let service = self.accounts_sync_service(&account_id);
        runtime::run(async move {
            let queued = match service {
                Some(service) => {
                    let queued = service.engine().load_waiting_headers().await?;
                    service.sync_now();
                    queued
                }
                None => mail_sync::load_waiting_headers_stored(&db).await?,
            };
            Ok(queued as u64)
        })
        .await
    }

    /// How many messages loading all of `account_id`'s mail over the
    /// Gmail API would download, and about how long it would take. Asks
    /// Gmail for its count (one cheap call); while Clean Up's header load
    /// waits for IMAP, the messages still waiting instead.
    pub async fn cleanup_load_estimate(&self, account_id: String) -> Result<CleanupLoadEstimate, CoreError> {
        let db = self.store_for(&account_id).await?;
        let Some(service) = self.accounts_sync_service(&account_id) else {
            return Ok(CleanupLoadEstimate { messages: None, seconds: None });
        };
        runtime::run(async move {
            if service.engine().headers_paused().await? {
                let (_, waiting) = db.read(mail_store::queue::counts).await?;
                return Ok(CleanupLoadEstimate {
                    messages: Some(waiting),
                    seconds: Some(provider_gmail::rest_download_seconds(waiting)),
                });
            }
            let total = match service.engine().provider().profile().await {
                Ok(profile) => profile.messages_total,
                Err(e) => {
                    tracing::info!(error = %e, "Gmail's message count is not available");
                    return Ok(CleanupLoadEstimate { messages: None, seconds: None });
                }
            };
            let stored: i64 = db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?)).await?;
            let messages = total.saturating_sub(stored.max(0) as u64);
            Ok(CleanupLoadEstimate {
                messages: Some(messages),
                seconds: Some(provider_gmail::rest_download_seconds(messages)),
            })
        })
        .await
    }
}
