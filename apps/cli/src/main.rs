//! `downpour` — the command-line front end and integration harness.
//!
//! M1 runs the engine in-process. A shared daemon (so `pause`/`cancel` act on a
//! download running in another process) arrives with the IPC server in M5;
//! today Ctrl-C pauses a foreground download and `downpour run <id>` resumes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use downpour_core::{CancelToken, Engine, EngineConfig, ProgressSample, ProgressSink, RunOutcome};
use downpour_store::downloads::DownloadRecord;
use downpour_types::{AddDownloadRequest, DownloadId, DownloadSource, DownloadState};

#[derive(Debug, Parser)]
#[command(name = "downpour", version, about = "Downpour download manager CLI")]
struct Cli {
    /// Database file (defaults to the per-user data directory).
    #[arg(long, global = true, env = "DOWNPOUR_DB")]
    db: Option<PathBuf>,

    /// Default download directory.
    #[arg(long, global = true, env = "DOWNPOUR_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Queue a download.
    Add(AddArgs),
    /// Queue a download and start it immediately.
    Get(AddArgs),
    /// Run queued or paused downloads in the foreground.
    Run {
        /// Download id; omit to run everything queued or paused.
        id: Option<String>,
        /// Do not print progress samples.
        #[arg(short, long)]
        quiet: bool,
    },
    /// List downloads.
    List {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Maximum rows.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show one download.
    Info {
        id: String,
        /// Print JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// Mark a queued download as paused.
    Pause { id: String },
    /// Cancel a download and delete its partial file.
    Cancel { id: String },
    /// Crawl a site and list (or queue) downloadable files.
    Grab {
        url: String,
        #[arg(long, default_value_t = 2)]
        depth: u32,
        #[arg(long, default_value_t = 100)]
        pages: u32,
        /// Follow links to other origins.
        #[arg(long)]
        cross_origin: bool,
        /// Ignore robots.txt (use only on sites you control).
        #[arg(long)]
        no_robots: bool,
        #[arg(long, default_value_t = 500)]
        delay_ms: u64,
        /// Queue every discovered file.
        #[arg(long)]
        add: bool,
    },
    /// Print engine and protocol version information.
    Version,
    /// Check the database, folders, tools, IPC, and browser registration.
    Doctor,
    /// Register native-messaging manifests for installed browsers.
    RegisterBrowsers,
}

#[derive(Debug, Args)]
struct AddArgs {
    url: String,
    /// Save as this file name.
    #[arg(short = 'n', long)]
    name: Option<String>,
    /// Destination directory (defaults to the configured download directory).
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// Parallel connections (used by the segmented engine in M2).
    #[arg(short, long, default_value_t = 8)]
    connections: u16,
    /// Extra request header, repeatable: -H "Referer: example.com/page".
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create async runtime")?;
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    let db = cli.db.clone();
    let dir = cli.dir.clone();

    match cli.command {
        Command::Version => {
            println!(
                "downpour {} (protocol {})",
                env!("CARGO_PKG_VERSION"),
                downpour_types::PROTOCOL_VERSION
            );
            Ok(())
        }
        Command::RegisterBrowsers => register_browsers(),
        Command::Doctor => doctor(&db, &dir).await,
        Command::List { json, limit } => list(&db, &dir, json, limit),
        Command::Info { id, json } => info(&db, &dir, &id, json),
        Command::Pause { id } => pause(&db, &dir, &id),
        Command::Cancel { id } => cancel(&db, &dir, &id),
        Command::Grab {
            url,
            depth,
            pages,
            cross_origin,
            no_robots,
            delay_ms,
            add,
        } => {
            grab(
                &db,
                &dir,
                url,
                depth,
                pages,
                cross_origin,
                no_robots,
                delay_ms,
                add,
            )
            .await
        }
        Command::Add(args) => {
            let mut engine = open_engine(&db, &dir, false)?;
            let id = queue(&mut engine, args)?;
            println!("queued {id}");
            Ok(())
        }
        Command::Get(args) => {
            let mut engine = open_engine(&db, &dir, true)?;
            let id = queue(&mut engine, args)?;
            run_downloads(&mut engine, Some(id), false).await
        }
        Command::Run { id, quiet } => {
            let mut engine = open_engine(&db, &dir, true)?;
            let id = match id {
                Some(raw) => Some(parse_id(&raw)?),
                None => None,
            };
            run_downloads(&mut engine, id, quiet).await
        }
    }
}

/// Open the engine. Only commands that will run transfers recover interrupted
/// rows; listing or pausing must not flip a download owned by another process.
const NM_HOST_NAME: &str = "com.downpour.nm";
/// Stable Chromium extension ID derived from the public key shipped in
/// `extensions/browser/chromium/manifest.json`.
const CHROME_EXTENSION_ID: &str = "lelbkegljeljlebagmalnnocmcmgpdjg";
/// Stable Firefox add-on id; must match `extensions/browser/firefox/manifest.json`.
const FIREFOX_EXTENSION_ID: &str = "downpour@yash244466666.github.io";

fn chromium_host_manifest(host: &Path) -> serde_json::Value {
    serde_json::json!({
        "name": NM_HOST_NAME,
        "description": "Downpour native messaging host",
        "path": host.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{CHROME_EXTENSION_ID}/")],
    })
}

fn firefox_host_manifest(host: &Path) -> serde_json::Value {
    serde_json::json!({
        "name": NM_HOST_NAME,
        "description": "Downpour native messaging host",
        "path": host.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    })
}

fn register_browsers() -> Result<()> {
    let host = locate_native_host()?;
    let chromium = serde_json::to_vec_pretty(&chromium_host_manifest(&host))?;
    let firefox = serde_json::to_vec_pretty(&firefox_host_manifest(&host))?;

    #[cfg(not(windows))]
    {
        let mut written = 0_usize;
        for base in browser_config_dirs() {
            if !base.exists() {
                continue;
            }
            let dir = base.join("NativeMessagingHosts");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{NM_HOST_NAME}.json"));
            std::fs::write(&path, &chromium)?;
            println!("wrote {}", path.display());
            written += 1;
        }
        for dir in firefox_manifest_dirs() {
            if !dir.parent().is_some_and(Path::exists) {
                continue;
            }
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{NM_HOST_NAME}.json"));
            std::fs::write(&path, &firefox)?;
            println!("wrote {}", path.display());
            written += 1;
        }
        if written == 0 {
            bail!(
                "no supported browser profile found; install Chrome, Chromium, Edge, \
                 Brave, or Firefox and retry"
            );
        }
    }

    #[cfg(windows)]
    {
        let manifest_dir = downpour_core::paths::data_dir().join("native-messaging");
        std::fs::create_dir_all(&manifest_dir)?;
        let path = manifest_dir.join(format!("{NM_HOST_NAME}.json"));
        std::fs::write(&path, &chromium)?;
        for key in [
            r"Software\Google\Chrome\NativeMessagingHosts\com.downpour.nm",
            r"Software\Chromium\NativeMessagingHosts\com.downpour.nm",
            r"Software\Microsoft\Edge\NativeMessagingHosts\com.downpour.nm",
            r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.downpour.nm",
        ] {
            let status = std::process::Command::new("reg")
                .args(["add", key, "/ve", "/t", "REG_SZ", "/d"])
                .arg(&path)
                .arg("/f")
                .status()?;
            if !status.success() {
                eprintln!("could not register {key}");
            }
        }

        let firefox_dir = windows_firefox_manifest_dir()?;
        std::fs::create_dir_all(&firefox_dir)?;
        let firefox_path = firefox_dir.join(format!("{NM_HOST_NAME}.json"));
        std::fs::write(&firefox_path, &firefox)?;
        let status = std::process::Command::new("reg")
            .args([
                "add",
                r"Software\Mozilla\NativeMessagingHosts\com.downpour.nm",
                "/ve",
                "/t",
                "REG_SZ",
                "/d",
            ])
            .arg(&firefox_path)
            .arg("/f")
            .status()?;
        if !status.success() {
            eprintln!("could not register the Firefox native messaging host");
        }
        println!("registered native messaging host for Chromium browsers and Firefox");
    }

    Ok(())
}

fn locate_native_host() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DOWNPOUR_NM_HOST") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "DOWNPOUR_NM_HOST does not point at a file: {}",
            path.display()
        );
    }

    let executable = std::env::current_exe().context("locate the CLI binary")?;
    let name = if cfg!(windows) {
        "downpour-nm-host.exe"
    } else {
        "downpour-nm-host"
    };
    if let Some(directory) = executable.parent() {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("cannot find {name}; build the workspace or set DOWNPOUR_NM_HOST")
}

#[cfg(not(windows))]
fn browser_config_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    browser_config_dirs_for(&home)
}

#[cfg(not(windows))]
fn browser_config_dirs_for(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support");
        vec![
            base.join("Google/Chrome"),
            base.join("Chromium"),
            base.join("Microsoft Edge"),
            base.join("BraveSoftware/Brave-Browser"),
        ]
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        vec![
            config.join("google-chrome"),
            config.join("chromium"),
            config.join("microsoft-edge"),
            config.join("BraveSoftware/Brave-Browser"),
        ]
    }
}

#[cfg(not(windows))]
fn firefox_manifest_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    firefox_manifest_dirs_for(&home)
}

/// Firefox reads native-messaging manifests from a fixed per-user directory.
#[cfg(not(windows))]
fn firefox_manifest_dirs_for(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Mozilla/NativeMessagingHosts")]
    } else {
        vec![home.join(".mozilla/native-messaging-hosts")]
    }
}

#[cfg(windows)]
fn windows_firefox_manifest_dir() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(appdata)
        .join("Mozilla")
        .join("NativeMessagingHosts"))
}

async fn doctor(db: &Option<PathBuf>, dir: &Option<PathBuf>) -> Result<()> {
    let mut failures = 0_usize;
    println!(
        "Downpour {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        downpour_types::PROTOCOL_VERSION
    );
    println!("{} / {}", std::env::consts::OS, std::env::consts::ARCH);
    println!();

    // Database.
    let db_path = db
        .clone()
        .unwrap_or_else(downpour_core::paths::database_path);
    match downpour_store::Store::open(&db_path) {
        Ok(store) => {
            let integrity: String = store
                .connection()
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap_or_else(|_| "unknown".to_owned());
            if integrity == "ok" {
                report(
                    true,
                    "database",
                    &format!("{} (integrity ok)", db_path.display()),
                );
            } else {
                failures += 1;
                report(
                    false,
                    "database",
                    &format!("integrity check returned {integrity}"),
                );
            }
        }
        Err(error) => {
            failures += 1;
            report(
                false,
                "database",
                &format!("{}: {error}", db_path.display()),
            );
        }
    }

    // Download folder.
    let download_dir = dir
        .clone()
        .unwrap_or_else(downpour_core::paths::default_download_dir);
    if let Err(error) = std::fs::create_dir_all(&download_dir) {
        failures += 1;
        report(
            false,
            "download folder",
            &format!("{}: {error}", download_dir.display()),
        );
    } else {
        let probe = download_dir.join(format!(".downpour-doctor-{}", std::process::id()));
        match std::fs::write(&probe, b"ok").and_then(|()| std::fs::remove_file(&probe)) {
            Ok(()) => report(true, "download folder", &download_dir.display().to_string()),
            Err(error) => {
                failures += 1;
                report(
                    false,
                    "download folder",
                    &format!("{}: {error}", download_dir.display()),
                );
            }
        }
    }

    // Tools.
    let tools = downpour_media::detect_tools();
    match &tools.yt_dlp {
        Some(path) => report(true, "yt-dlp", &path.display().to_string()),
        None => report_warn(
            "yt-dlp",
            "not found — HLS uses the built-in fallback; DASH is unavailable",
        ),
    }
    match &tools.ffmpeg {
        Some(path) => report(true, "ffmpeg", &path.display().to_string()),
        None => report_warn(
            "ffmpeg",
            "not found — remuxing and DASH mixing are unavailable",
        ),
    }

    // IPC engine.
    match downpour_ipc::IpcClient::connect(downpour_ipc::ClientKind::Cli).await {
        Ok(mut client) => {
            let request = downpour_ipc::Request {
                id: 1,
                method: "ping".to_owned(),
                params: serde_json::json!({}),
            };
            match client.call(&request).await {
                Ok(_) => report(true, "engine IPC", "running and reachable"),
                Err(error) => report_warn("engine IPC", &format!("connection failed: {error}")),
            }
        }
        Err(_) => report_warn("engine IPC", "app not running (start Downpour to test)"),
    }

    // Browser registration.
    #[cfg(not(windows))]
    {
        let mut registered = 0_usize;
        for base in browser_config_dirs() {
            let path = base
                .join("NativeMessagingHosts")
                .join(format!("{NM_HOST_NAME}.json"));
            if !path.is_file() {
                continue;
            }
            registered += 1;
            report_browser_manifest(&path);
        }
        for dir in firefox_manifest_dirs() {
            let path = dir.join(format!("{NM_HOST_NAME}.json"));
            if !path.is_file() {
                continue;
            }
            registered += 1;
            report_browser_manifest(&path);
        }
        if registered == 0 {
            report_warn(
                "browser manifest",
                "not registered (run `downpour register-browsers`)",
            );
        }
    }

    println!();
    if failures > 0 {
        bail!("{failures} check(s) failed");
    }
    println!("all checks passed");
    Ok(())
}

fn report(ok: bool, label: &str, detail: &str) {
    let mark = if ok { "ok  " } else { "FAIL" };
    println!("  {mark}  {label:<18} {detail}");
}

#[cfg(not(windows))]
fn report_browser_manifest(path: &Path) {
    match std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|value| {
            value
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }) {
        Some(host) if Path::new(&host).is_file() => {
            report(
                true,
                "browser manifest",
                &format!("{} -> {host}", path.display()),
            );
        }
        Some(host) => report_warn(
            "browser manifest",
            &format!("{} points at a missing host: {host}", path.display()),
        ),
        None => report_warn(
            "browser manifest",
            &format!("unreadable: {}", path.display()),
        ),
    }
}

fn report_warn(label: &str, detail: &str) {
    println!("  warn  {label:<18} {detail}");
}

struct StoreFrontier<'a> {
    store: &'a mut downpour_store::Store,
    job_id: &'a str,
}

impl downpour_grabber::Frontier for StoreFrontier<'_> {
    fn discovered(&mut self, url: &str, depth: u32) {
        let _ = downpour_store::grabber::upsert_frontier(
            self.store.connection(),
            self.job_id,
            url,
            depth,
            "pending",
        );
    }

    fn visited(&mut self, url: &str) {
        let _ = downpour_store::grabber::set_frontier_state(
            self.store.connection(),
            self.job_id,
            url,
            "visited",
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn grab(
    db: &Option<PathBuf>,
    dir: &Option<PathBuf>,
    url: String,
    depth: u32,
    pages: u32,
    cross_origin: bool,
    no_robots: bool,
    delay_ms: u64,
    add: bool,
) -> Result<()> {
    let seed = url::Url::parse(&url).context("invalid seed URL")?;
    if !matches!(seed.scheme(), "http" | "https") {
        bail!("only http:// and https:// seeds are supported");
    }

    let db_path = db
        .clone()
        .unwrap_or_else(downpour_core::paths::database_path);
    let mut store = downpour_store::Store::open(&db_path).context("open database")?;
    let job_id = DownloadId::new().to_string();
    downpour_store::grabber::insert_job(
        store.connection(),
        &downpour_store::grabber::NewGrabJob {
            id: job_id.clone(),
            seed_url: seed.to_string(),
            filters_json: None,
            max_depth: depth,
            max_pages: pages,
            same_origin: !cross_origin,
            respect_robots: !no_robots,
            created_at_ms: downpour_store::downloads::now_ms(),
        },
    )
    .context("create grabber job")?;

    let options = downpour_grabber::GrabOptions {
        max_depth: depth,
        max_pages: pages,
        same_origin: !cross_origin,
        respect_robots: !no_robots,
        delay: std::time::Duration::from_millis(delay_ms),
        ..downpour_grabber::GrabOptions::default()
    };
    let client = reqwest::Client::builder()
        .user_agent(format!("Downpour/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build crawler client")?;

    let result = {
        let mut frontier = StoreFrontier {
            store: &mut store,
            job_id: &job_id,
        };
        downpour_grabber::crawl(&client, seed, &options, &mut frontier, &|| false, |visit| {
            eprintln!(
                "[{}] {} — {} links, {} files",
                visit.depth, visit.url, visit.links, visit.files
            );
        })
        .await
        .context("crawl site")?
    };

    downpour_store::grabber::update_job(
        store.connection(),
        &job_id,
        "completed",
        result.pages.len() as u32,
        result.files.len() as u32,
    )
    .context("update grabber job")?;

    println!("{} pages, {} files", result.pages.len(), result.files.len());
    for file in &result.files {
        println!("{}", file.url);
    }

    if add {
        let mut engine = open_engine(db, dir, false)?;
        for file in result.files {
            let id = engine.add(AddDownloadRequest {
                url: file.url,
                ..AddDownloadRequest::default()
            })?;
            println!("queued {id}");
        }
    }
    Ok(())
}

fn open_engine(db: &Option<PathBuf>, dir: &Option<PathBuf>, recover: bool) -> Result<Engine> {
    let db = db
        .clone()
        .unwrap_or_else(downpour_core::paths::database_path);
    let default_dir = dir
        .clone()
        .unwrap_or_else(downpour_core::paths::default_download_dir);

    let config = EngineConfig {
        default_dir,
        ..EngineConfig::default()
    };
    let engine = if recover {
        Engine::open(db, config)
    } else {
        Engine::open_without_recovery(db, config)
    };
    engine.context("open engine database")
}

fn queue(engine: &mut Engine, args: AddArgs) -> Result<DownloadId> {
    let headers = parse_headers(&args.headers)?;
    let request = AddDownloadRequest {
        url: args.url,
        file_name: args.name,
        dest_dir: args.output.map(|path| path.to_string_lossy().into_owned()),
        connections: Some(args.connections),
        headers,
        source: DownloadSource::Api,
        ..AddDownloadRequest::default()
    };

    engine.add(request).context("queue download")
}

fn parse_headers(raw: &[String]) -> Result<BTreeMap<String, String>> {
    let mut headers = BTreeMap::new();
    for item in raw {
        let (name, value) = item
            .split_once(':')
            .with_context(|| format!("header must be \"Name: value\": {item}"))?;
        if name.trim().is_empty() {
            bail!("header name is empty: {item}");
        }
        headers.insert(name.trim().to_owned(), value.trim().to_owned());
    }
    Ok(headers)
}

fn parse_id(raw: &str) -> Result<DownloadId> {
    DownloadId::from_str(raw).with_context(|| format!("invalid download id: {raw}"))
}

async fn run_downloads(engine: &mut Engine, id: Option<DownloadId>, quiet: bool) -> Result<()> {
    let ids: Vec<DownloadId> = match id {
        Some(id) => vec![id],
        None => {
            let runnable = engine.list_runnable().context("list downloads")?;
            if runnable.is_empty() {
                println!("nothing queued or paused");
                return Ok(());
            }
            runnable.into_iter().map(|record| record.id).collect()
        }
    };

    let mut failures = 0_usize;
    for id in ids {
        let record = engine.get(id).context("load download")?;
        let Some(record) = record else {
            eprintln!("{id}: no such download");
            failures += 1;
            continue;
        };

        if !quiet {
            println!("{} -> {}", record.file_name, record.url);
        }

        let cancel = CancelToken::new();
        let watcher = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancel.cancel();
                }
            }
        });

        let sink = |sample: ProgressSample| print_progress(&sample);
        let progress: Option<ProgressSink<'_>> = if quiet { None } else { Some(&sink) };
        let outcome = engine.run(id, cancel, progress).await;
        watcher.abort();

        if !quiet {
            println!();
        }

        match outcome {
            Ok(RunOutcome::Completed) => {
                let record = engine.get(id).context("reload download")?;
                if let Some(record) = record {
                    println!(
                        "completed {} ({})",
                        record.file_name,
                        human_bytes(record.downloaded_bytes)
                    );
                }
            }
            Ok(RunOutcome::Paused) => {
                println!("paused {id}; resume with: downpour run {id}");
            }
            Err(error) => {
                eprintln!("{id}: {error}");
                failures += 1;
            }
        }
    }

    if failures > 0 {
        bail!("{failures} download(s) failed");
    }
    Ok(())
}

fn list(db: &Option<PathBuf>, dir: &Option<PathBuf>, json: bool, limit: usize) -> Result<()> {
    let engine = open_engine(db, dir, false)?;
    let records = engine.list(limit).context("list downloads")?;

    if json {
        for record in &records {
            println!("{}", serde_json::to_string(record).unwrap_or_default());
        }
        return Ok(());
    }

    if records.is_empty() {
        println!("no downloads");
        return Ok(());
    }

    println!(
        "{:<10} {:>10} {:>12}  {:<32}  ID",
        "STATE", "SIZE", "PROGRESS", "NAME"
    );
    for record in records {
        println!(
            "{:<10} {:>10} {:>12}  {:<32}  {}",
            record.state.as_str(),
            record
                .total_bytes
                .map(human_bytes)
                .unwrap_or_else(|| "?".to_owned()),
            progress_label(&record),
            truncate(&record.file_name, 32),
            record.id
        );
    }
    Ok(())
}

fn info(db: &Option<PathBuf>, dir: &Option<PathBuf>, raw_id: &str, json: bool) -> Result<()> {
    let engine = open_engine(db, dir, false)?;
    let id = parse_id(raw_id)?;
    let record = engine
        .get(id)
        .context("load download")?
        .with_context(|| format!("no such download: {id}"))?;

    if json {
        println!("{}", serde_json::to_string(&record).unwrap_or_default());
        return Ok(());
    }

    println!("id:          {}", record.id);
    println!("url:         {}", record.url);
    if let Some(final_url) = &record.final_url {
        println!("final url:   {final_url}");
    }
    println!("name:        {}", record.file_name);
    println!("directory:   {}", record.dest_dir);
    println!("state:       {}", record.state.as_str());
    println!(
        "size:        {}",
        record
            .total_bytes
            .map(human_bytes)
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!("downloaded:  {}", human_bytes(record.downloaded_bytes));
    println!(
        "ranges:      {}",
        match record.supports_ranges {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        }
    );
    if let Some(etag) = &record.etag {
        println!("etag:        {etag}");
    }
    if let Some(final_path) = &record.final_path {
        println!("path:        {final_path}");
    }
    if record.state != DownloadState::Completed {
        if let Some(temp_path) = &record.temp_path {
            println!("partial:     {temp_path}");
        }
    }
    if let Some(checksum) = &record.checksum_sha256 {
        println!("sha256:      {checksum}");
    }
    if let Some(error) = &record.error {
        println!("error:       {error}");
    }
    Ok(())
}

fn pause(db: &Option<PathBuf>, dir: &Option<PathBuf>, raw_id: &str) -> Result<()> {
    let mut engine = open_engine(db, dir, false)?;
    let id = parse_id(raw_id)?;
    engine.pause(id).context("pause download")?;
    println!("paused {id}");
    Ok(())
}

fn cancel(db: &Option<PathBuf>, dir: &Option<PathBuf>, raw_id: &str) -> Result<()> {
    let mut engine = open_engine(db, dir, false)?;
    let id = parse_id(raw_id)?;
    engine.cancel(id).context("cancel download")?;
    println!("canceled {id}");
    Ok(())
}

fn print_progress(sample: &ProgressSample) {
    use std::io::Write;

    let line = match sample.total_bytes {
        Some(total) if total > 0 => {
            let percent = sample.downloaded_bytes as f64 / total as f64 * 100.0;
            format!(
                "\r{} / {}  {:>5.1}%  {:>10}",
                human_bytes(sample.downloaded_bytes),
                human_bytes(total),
                percent,
                human_rate(sample.speed_bps)
            )
        }
        _ => format!(
            "\r{}  {:>10}",
            human_bytes(sample.downloaded_bytes),
            human_rate(sample.speed_bps)
        ),
    };
    print!("{line:<78}");
    let _ = std::io::stdout().flush();
}

fn progress_label(record: &DownloadRecord) -> String {
    match (record.state, record.total_bytes) {
        (DownloadState::Completed, _) => "done".to_owned(),
        (_, Some(total)) if total > 0 => {
            format!(
                "{:.1}%",
                record.downloaded_bytes as f64 / total as f64 * 100.0
            )
        }
        _ => human_bytes(record.downloaded_bytes),
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn human_rate(bytes_per_second: u64) -> String {
    format!("{}/s", human_bytes(bytes_per_second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_manifest_lists_the_pinned_extension() {
        let manifest = chromium_host_manifest(Path::new("/opt/downpour-nm-host"));
        assert_eq!(manifest["name"], NM_HOST_NAME);
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(
            manifest["allowed_origins"][0],
            format!("chrome-extension://{CHROME_EXTENSION_ID}/")
        );
    }

    #[test]
    fn firefox_manifest_uses_allowed_extensions() {
        let manifest = firefox_host_manifest(Path::new("/opt/downpour-nm-host"));
        assert_eq!(manifest["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
        assert!(manifest.get("allowed_origins").is_none());
    }

    #[test]
    fn firefox_id_matches_the_extension_manifest() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions/browser/firefox/manifest.json");
        let body = std::fs::read_to_string(path).expect("read the Firefox manifest");
        let manifest: serde_json::Value =
            serde_json::from_str(&body).expect("parse the Firefox manifest");
        assert_eq!(
            manifest["browser_specific_settings"]["gecko"]["id"],
            FIREFOX_EXTENSION_ID
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn firefox_manifest_dir_matches_the_platform() {
        let dirs = firefox_manifest_dirs_for(Path::new("/home/tester"));
        assert_eq!(dirs.len(), 1);
        if cfg!(target_os = "macos") {
            assert_eq!(
                dirs[0],
                Path::new("/home/tester/Library/Application Support/Mozilla/NativeMessagingHosts")
            );
        } else {
            assert_eq!(
                dirs[0],
                Path::new("/home/tester/.mozilla/native-messaging-hosts")
            );
        }
    }
}
