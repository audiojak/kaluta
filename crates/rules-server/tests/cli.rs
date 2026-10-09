//! The operator's commands, run as the binary: a backup while the
//! database is open, forgetting a mailbox, and the health check.

mod common;

use std::process::Command;

use common::{MAILBOX, snapshot, start};

fn rules(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_openagc-rules")).args(args).output().expect("run openagc-rules")
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_forget_and_healthcheck() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let dir = s.dir.to_str().unwrap();

    let copy = s.dir.join("copy.sqlite3");
    let out = rules(&["backup", copy.to_str().unwrap(), "--data-dir", dir]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the copy is its owner's only, as the database is");
    }
    let backup = rusqlite::Connection::open(&copy).unwrap();
    let n: i64 = backup.query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1, "the copy holds what was published");
    assert!(!rules(&["backup", copy.to_str().unwrap(), "--data-dir", dir]).status.success(), "never overwrites");

    let out = rules(&["forget-mailbox", MAILBOX, "--data-dir", dir]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let (status, _) = s.register(MAILBOX).await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "registered again after being forgotten");
    assert!(!rules(&["forget-mailbox", "nobody@x.example", "--data-dir", dir]).status.success());

    let listen = s.base.trim_start_matches("http://");
    assert!(rules(&["healthcheck", "--listen", listen]).status.success());
    assert!(!rules(&["healthcheck", "--listen", "127.0.0.1:9"]).status.success());
    assert_eq!(rules(&["--bogus"]).status.code(), Some(2));
}
