//! `openagc-rules`: the rules server for cloud agents (spec §10.6).
//!
//! ```text
//! openagc-rules [serve] [--listen <addr>] [--data-dir <dir>] [--log <filter>] [--rate-limit <n>] [--public-url <url>] [--trusted-proxy <cidr>]
//! openagc-rules forget-mailbox <address> [--data-dir <dir>]
//! openagc-rules backup <file> [--data-dir <dir>]
//! openagc-rules healthcheck [--listen <addr>]
//! ```
//!
//! Each flag has an environment variable, which the flag overrides:
//! `OPENAGC_RULES_LISTEN` (default `127.0.0.1:8787`), `OPENAGC_RULES_DATA_DIR`
//! (default `./data`), `OPENAGC_RULES_LOG` (default `info`),
//! `OPENAGC_RULES_RATE_LIMIT` (requests per minute per token, default 120),
//! `OPENAGC_RULES_PUBLIC_URL` (the server's https:// origin as agents reach
//! it; turns on OAuth sign-in with connect codes),
//! `OPENAGC_RULES_TRUSTED_PROXY` (the TLS proxy's address or CIDR network,
//! comma-separated, whose `X-Forwarded-For` names the client; the flag
//! repeats).
//! `OPENAGC_RULES_REGISTRATION_TOKEN`, only from the environment, makes
//! registering a mailbox need that bearer token.
//! `OPENAGC_RULES_REQUIRE_ENCRYPTION=1` refuses plaintext snapshots (the
//! project-hosted server sets it; spec §10.6, encryption at rest).
//!
//! Plain HTTP: put it behind a TLS proxy (`docs/rules-server.md`).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use rules_server::{Config, Db, db};

const USAGE: &str =
    "usage: openagc-rules [serve] [--listen <addr>] [--data-dir <dir>] [--log <filter>] [--rate-limit <n>] [--public-url <url>] [--trusted-proxy <cidr>]
       openagc-rules forget-mailbox <address> [--data-dir <dir>]
       openagc-rules backup <file> [--data-dir <dir>]
       openagc-rules healthcheck [--listen <addr>]";

const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

enum Command {
    Serve,
    Forget(String),
    Backup(PathBuf),
    Healthcheck,
}

struct Args {
    command: Command,
    listen: String,
    data_dir: PathBuf,
    log: String,
    rate_limit: u32,
    public_url: Option<String>,
    trusted_proxies: Vec<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        command: Command::Serve,
        listen: env("OPENAGC_RULES_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.into()),
        data_dir: env("OPENAGC_RULES_DATA_DIR").map_or_else(|| PathBuf::from("data"), PathBuf::from),
        log: env("OPENAGC_RULES_LOG").unwrap_or_else(|| "info".into()),
        rate_limit: 120,
        public_url: env("OPENAGC_RULES_PUBLIC_URL"),
        trusted_proxies: env("OPENAGC_RULES_TRUSTED_PROXY").into_iter().collect(),
    };
    if let Some(v) = env("OPENAGC_RULES_RATE_LIMIT") {
        args.rate_limit = v.parse().map_err(|_| format!("OPENAGC_RULES_RATE_LIMIT {v:?} is not a number"))?;
    }
    let mut it = std::env::args().skip(1).peekable();
    match it.peek().map(String::as_str) {
        Some("serve") => {
            it.next();
        }
        Some("forget-mailbox") => {
            it.next();
            args.command = Command::Forget(it.next().ok_or("forget-mailbox needs an address")?);
        }
        Some("backup") => {
            it.next();
            args.command = Command::Backup(PathBuf::from(it.next().ok_or("backup needs a file to write")?));
        }
        Some("healthcheck") => {
            it.next();
            args.command = Command::Healthcheck;
        }
        _ => {}
    }
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--listen" => args.listen = value()?,
            "--data-dir" => args.data_dir = PathBuf::from(value()?),
            "--log" => args.log = value()?,
            "--public-url" => args.public_url = Some(value()?),
            "--trusted-proxy" => args.trusted_proxies.push(value()?),
            "--rate-limit" => {
                let v = value()?;
                args.rate_limit = v.parse().map_err(|_| format!("--rate-limit {v:?} is not a number"))?;
            }
            "--version" => {
                println!("openagc-rules {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("openagc-rules: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match &args.command {
        Command::Serve => serve(&args),
        Command::Forget(address) => forget(&args, address),
        Command::Backup(file) => backup(&args, file),
        Command::Healthcheck => healthcheck(&args.listen),
    }
}

fn serve(args: &Args) -> ExitCode {
    // Dependencies log at warn: what they say at info and below can carry
    // request contents. The server's own lines never do.
    let filter = tracing_subscriber::EnvFilter::try_new(format!("warn,rules_server={0},openagc_rules={0}", args.log))
        .or_else(|_| tracing_subscriber::EnvFilter::try_new(&args.log));
    let filter = match filter {
        Ok(f) => f,
        Err(e) => {
            eprintln!("openagc-rules: --log {:?}: {e}", args.log);
            return ExitCode::from(2);
        }
    };
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let config = Config {
        data_dir: args.data_dir.clone(),
        rate_limit_per_minute: args.rate_limit,
        registration_token: env("OPENAGC_RULES_REGISTRATION_TOKEN"),
        public_url: args.public_url.clone(),
        require_encryption: env("OPENAGC_RULES_REQUIRE_ENCRYPTION")
            .is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes")),
        trusted_proxies: args.trusted_proxies.clone(),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("openagc-rules: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        let app = match rules_server::app(&config) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, "cannot start");
                return ExitCode::FAILURE;
            }
        };
        let listener = match tokio::net::TcpListener::bind(&args.listen).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(listen = %args.listen, error = %e, "cannot listen");
                return ExitCode::FAILURE;
            }
        };
        tracing::info!(
            listen = %args.listen,
            data_dir = %config.data_dir.display(),
            rate_limit = config.rate_limit_per_minute,
            registration_open = config.registration_token.is_none(),
            oauth = config.public_url.as_deref().unwrap_or("off"),
            trusted_proxies = config.trusted_proxies.join(","),
            "openagc-rules {} serving",
            env!("CARGO_PKG_VERSION")
        );
        match axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown())
            .await
        {
            Ok(()) => {
                tracing::info!("stopped");
                ExitCode::SUCCESS
            }
            Err(e) => {
                tracing::error!(error = %e, "server failed");
                ExitCode::FAILURE
            }
        }
    })
}

/// Ctrl-C, or SIGTERM from `docker stop`.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}

/// For the operator: forget a mailbox whose publisher token is lost, so the
/// app can register it again.
fn forget(args: &Args, address: &str) -> ExitCode {
    let address = address.trim().to_lowercase();
    let result = Db::open(&args.data_dir).and_then(|db| {
        db.run_now(|c| match db::mailbox_by_address(c, &address)? {
            Some(m) => {
                db::delete_mailbox(c, m.id)?;
                db::scrub(c)?;
                Ok(true)
            }
            None => Ok(false),
        })
    });
    match result {
        Ok(true) => {
            println!("forgot {address}, its snapshots and its agent tokens");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("openagc-rules: {address} is not registered here");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("openagc-rules: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A consistent copy of the database while the server runs (SQLite's
/// `VACUUM INTO`), for when there is no `sqlite3` at hand (the image). The
/// copy is made readable by its owner only, as the database is.
fn backup(args: &Args, file: &std::path::Path) -> ExitCode {
    // Made empty first, with its mode, whatever the umask: `VACUUM INTO`
    // fills an empty file and refuses one with anything in it.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Err(e) = options.open(file) {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            eprintln!("openagc-rules: {} exists; give a new file", file.display());
        } else {
            eprintln!("openagc-rules: {}: {e}", file.display());
        }
        return ExitCode::FAILURE;
    }
    let target = file.to_string_lossy().into_owned();
    let copied = Db::open(&args.data_dir).and_then(|db| db.run_now(|c| c.execute("VACUUM INTO ?1", [&target])));
    #[cfg(unix)]
    if copied.is_ok() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    if copied.is_err() {
        let _ = std::fs::remove_file(file);
    }
    match copied {
        Ok(_) => {
            println!("backed up to {}", file.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("openagc-rules: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `GET /healthz` on the listen address, for a container's health check
/// (the image has no curl). An unspecified address is reached on loopback.
fn healthcheck(listen: &str) -> ExitCode {
    let addr: Option<SocketAddr> = listen.to_socket_addrs().ok().and_then(|mut a| a.next()).map(|mut a| {
        if a.ip().is_unspecified() {
            a.set_ip(if a.is_ipv4() { [127, 0, 0, 1].into() } else { std::net::Ipv6Addr::LOCALHOST.into() });
        }
        a
    });
    let Some(addr) = addr else {
        eprintln!("openagc-rules: cannot resolve {listen}");
        return ExitCode::FAILURE;
    };
    let check = || -> std::io::Result<bool> {
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        s.write_all(format!("GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())?;
        let mut reply = String::new();
        s.read_to_string(&mut reply)?;
        Ok(reply.starts_with("HTTP/1.1 200"))
    };
    match check() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("openagc-rules: unhealthy");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("openagc-rules: {e}");
            ExitCode::FAILURE
        }
    }
}
