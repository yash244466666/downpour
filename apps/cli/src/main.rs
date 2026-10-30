//! `downpour` — the command-line front end and integration harness.
//!
//! M1 runs the engine in-process. A shared daemon (so `pause`/`cancel` act on a
//! download running in another process) arrives with the IPC server in M5;
//! today Ctrl-C pauses a foreground download and `downpour run <id>` resumes it.

use std::collections::BTreeMap;
use std::path::PathBuf;
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
    /// Print engine and protocol version information.
    Version,
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
        Command::RegisterBrowsers => {
            bail!("browser registration arrives with the native-messaging host (M5)")
        }
        Command::List { json, limit } => list(&db, &dir, json, limit),
        Command::Info { id, json } => info(&db, &dir, &id, json),
        Command::Pause { id } => pause(&db, &dir, &id),
        Command::Cancel { id } => cancel(&db, &dir, &id),
        Command::Add(args) => {
            let mut engine = open_engine(&db, &dir)?;
            let id = queue(&mut engine, args)?;
            println!("queued {id}");
            Ok(())
        }
        Command::Get(args) => {
            let mut engine = open_engine(&db, &dir)?;
            let id = queue(&mut engine, args)?;
            run_downloads(&mut engine, Some(id), false).await
        }
        Command::Run { id, quiet } => {
            let mut engine = open_engine(&db, &dir)?;
            let id = match id {
                Some(raw) => Some(parse_id(&raw)?),
                None => None,
            };
            run_downloads(&mut engine, id, quiet).await
        }
    }
}

fn open_engine(db: &Option<PathBuf>, dir: &Option<PathBuf>) -> Result<Engine> {
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
    Engine::open(db, config).context("open engine database")
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
    let engine = open_engine(db, dir)?;
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
    let engine = open_engine(db, dir)?;
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
    let mut engine = open_engine(db, dir)?;
    let id = parse_id(raw_id)?;
    engine.pause(id).context("pause download")?;
    println!("paused {id}");
    Ok(())
}

fn cancel(db: &Option<PathBuf>, dir: &Option<PathBuf>, raw_id: &str) -> Result<()> {
    let mut engine = open_engine(db, dir)?;
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
