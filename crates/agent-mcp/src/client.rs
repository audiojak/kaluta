//! The shim's end of the socket.

use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, oneshot};

use crate::wire::{
    CallReply, CallRequest, Hello, HelloReply, Outcome, PROTOCOL_VERSION, WireError, read_frame, write_frame,
};

type Pending = Arc<std::sync::Mutex<Option<HashMap<u64, oneshot::Sender<Outcome>>>>>;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot reach Kaluta: {0}")]
    Connect(#[from] WireError),
    #[error("Kaluta refused the session: {0}")]
    Refused(String),
    /// The socket (or the directory it is in) could have been put there by
    /// someone else: nothing is sent to it.
    #[error("not connecting to {0}")]
    Untrusted(String),
}

/// This process's effective user id.
fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Whether `socket` is one this user's app made, as far as the file system
/// can say: a socket (not a link to one) owned by `uid`, in a directory
/// owned by `uid` that no one else can write to (so no one else can have
/// put it there or swap it), whose own parent, if others can write to it,
/// is sticky (like `/tmp`, where they cannot rename our directory away).
/// Mailbox mode reads the socket's path from a file (spec §10.1), and the
/// app may use `/tmp/kaluta-<uid>` when its own folder's path is too long,
/// which another user could create first after a restart.
pub fn check_socket(socket: &Path, uid: u32) -> Result<(), String> {
    let shown = socket.display();
    let meta = std::fs::symlink_metadata(socket).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.file_type().is_socket() {
        return Err(format!("{shown} is not a socket"));
    }
    if meta.uid() != uid {
        return Err(format!("{shown} belongs to another user"));
    }
    let dir = socket.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let dir_meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !dir_meta.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    if dir_meta.uid() != uid {
        return Err(format!("{} belongs to another user", dir.display()));
    }
    if dir_meta.mode() & 0o022 != 0 {
        return Err(format!("{} can be written by other users", dir.display()));
    }
    if let Some(up) = dir.parent().filter(|d| !d.as_os_str().is_empty())
        && let Ok(up_meta) = std::fs::metadata(up)
        && up_meta.mode() & 0o022 != 0
        && up_meta.mode() & 0o1000 == 0
    {
        return Err(format!("{} can be written by other users", up.display()));
    }
    Ok(())
}

/// A connection bound to one agent session. Calls may run concurrently.
pub struct ShimClient {
    writer: Mutex<OwnedWriteHalf>,
    pending: Pending,
    next_id: AtomicU64,
}

impl ShimClient {
    pub async fn connect(socket: &Path, session: &str) -> Result<Self, ClientError> {
        let hello = Hello { protocol: PROTOCOL_VERSION, session: session.to_owned(), mailbox: None, client: None };
        Self::open(socket, hello).await
    }

    /// Mailbox mode: an outside agent's session on the agent mailbox at
    /// `mailbox` (spec §10.1). Calls name mailbox tools. The socket's path
    /// comes from a file, so it is checked first ([`check_socket`]), and the
    /// process answering must run as this user.
    pub async fn connect_mailbox(socket: &Path, mailbox: &str, client: &str) -> Result<Self, ClientError> {
        let uid = current_uid();
        check_socket(socket, uid).map_err(ClientError::Untrusted)?;
        let stream = UnixStream::connect(socket).await.map_err(WireError::from)?;
        let peer = stream.peer_cred().map(|c| c.uid()).ok();
        if peer != Some(uid) {
            return Err(ClientError::Untrusted(format!("{}: it is served by another user", socket.display())));
        }
        let hello = Hello {
            protocol: PROTOCOL_VERSION,
            session: String::new(),
            mailbox: Some(mailbox.to_owned()),
            client: Some(client.to_owned()),
        };
        Self::start(stream, hello).await
    }

    async fn open(socket: &Path, hello: Hello) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).await.map_err(WireError::from)?;
        Self::start(stream, hello).await
    }

    async fn start(stream: UnixStream, hello: Hello) -> Result<Self, ClientError> {
        let (mut reader, mut writer) = stream.into_split();
        write_frame(&mut writer, &hello).await?;
        let reply: HelloReply = read_frame(&mut reader)
            .await?
            .ok_or_else(|| ClientError::Refused("the app closed the connection".into()))?;
        if !reply.ok {
            return Err(ClientError::Refused(reply.error.unwrap_or_default()));
        }
        let pending: Pending = Arc::new(std::sync::Mutex::new(Some(HashMap::new())));
        let routes = pending.clone();
        tokio::spawn(async move {
            while let Ok(Some(reply)) = read_frame::<_, CallReply>(&mut reader).await {
                let waiter =
                    routes.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&reply.id));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(reply.outcome);
                }
            }
            // The app went away: fail everything still waiting, and any
            // later call, instead of hanging the agent.
            routes.lock().unwrap_or_else(|e| e.into_inner()).take();
        });
        Ok(Self { writer: Mutex::new(writer), pending, next_id: AtomicU64::new(1) })
    }

    /// Whether the app has gone away (later calls answer `app_unavailable`).
    pub fn is_closed(&self) -> bool {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).is_none()
    }

    pub async fn call(&self, tool: &str, arguments: serde_json::Value) -> Outcome {
        let unavailable = || Outcome::error("app_unavailable", "Kaluta is not running or closed this session");
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            match pending.as_mut() {
                Some(map) => {
                    map.insert(id, tx);
                }
                None => return unavailable(),
            }
        }
        let request = CallRequest { id, tool: tool.to_owned(), arguments };
        if write_frame(&mut *self.writer.lock().await, &request).await.is_err() {
            return unavailable();
        }
        rx.await.unwrap_or_else(|_| unavailable())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A private scratch folder under `/tmp` (short enough for a socket).
    fn dir(name: &str) -> Dir {
        let dir = PathBuf::from(format!("/tmp/oagc-sock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        Dir(dir)
    }

    #[tokio::test]
    async fn our_own_socket_in_a_private_folder_is_used() {
        let d = dir("ok");
        let path = d.0.join("s.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let uid = current_uid();
        assert_eq!(check_socket(&path, uid), Ok(()));
        // The connection goes through the checks to the hello.
        let accept = tokio::spawn(async move {
            let (stream, _) = _listener.accept().await.unwrap();
            let (mut r, mut w) = stream.into_split();
            let _: Option<Hello> = read_frame(&mut r).await.unwrap();
            write_frame(&mut w, &HelloReply { ok: false, error: Some("no such mailbox".into()) }).await.unwrap();
        });
        let refused = ShimClient::connect_mailbox(&path, "a@b.example", "test").await.err().unwrap();
        assert!(matches!(&refused, ClientError::Refused(why) if why == "no such mailbox"), "{refused}");
        accept.await.unwrap();
    }

    #[tokio::test]
    async fn a_socket_others_could_have_placed_is_refused() {
        let uid = current_uid();
        // Someone else's socket (as far as the check can tell).
        let d = dir("owner");
        let path = d.0.join("s.sock");
        let _l = tokio::net::UnixListener::bind(&path).unwrap();
        assert_eq!(check_socket(&path, uid + 1), Err(format!("{} belongs to another user", path.display())));

        // A folder other users can write to (no sticky bit).
        let open = dir("open");
        let path = open.0.join("s.sock");
        let _l2 = tokio::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&open.0, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(check_socket(&path, uid), Err(format!("{} can be written by other users", open.0.display())));
        let err = ShimClient::connect_mailbox(&path, "a@b.example", "test").await.err().unwrap();
        assert!(matches!(err, ClientError::Untrusted(_)), "{err}");
        assert!(err.to_string().contains("can be written by other users"), "{err}");
        std::fs::set_permissions(&open.0, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(check_socket(&path, uid).is_err(), "group-writable too");

        // A link to a good socket, and a plain file, are not sockets.
        let links = dir("link");
        let good = links.0.join("good.sock");
        let _l3 = tokio::net::UnixListener::bind(&good).unwrap();
        let link = links.0.join("link.sock");
        std::os::unix::fs::symlink(&good, &link).unwrap();
        assert_eq!(check_socket(&link, uid), Err(format!("{} is not a socket", link.display())));
        let file = links.0.join("file.sock");
        std::fs::write(&file, "").unwrap();
        assert_eq!(check_socket(&file, uid), Err(format!("{} is not a socket", file.display())));
        // A folder that is a link is refused too.
        let via = PathBuf::from(format!("/tmp/oagc-sock-via-{}", std::process::id()));
        let _ = std::fs::remove_file(&via);
        std::os::unix::fs::symlink(&links.0, &via).unwrap();
        let through = via.join("good.sock");
        assert_eq!(check_socket(&through, uid), Err(format!("{} is not a directory", via.display())));
        std::fs::remove_file(&via).unwrap();
    }
}
