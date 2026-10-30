use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "downpour", version, about = "Downpour download manager CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add a download by URL.
    Add {
        url: String,
        /// Parallel connections (1-32).
        #[arg(short, long, default_value_t = 8)]
        connections: u16,
    },
    /// List downloads.
    List,
    /// Show engine and protocol version information.
    Info,
    /// Register native-messaging manifests for installed browsers.
    RegisterBrowsers,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Add { url, connections } => {
            anyhow::bail!(
                "engine IPC is not wired yet; cannot add {url} with {connections} connections"
            );
        }
        Command::List => anyhow::bail!("engine IPC is not wired yet"),
        Command::Info => {
            println!(
                "downpour {} (protocol {})",
                env!("CARGO_PKG_VERSION"),
                downpour_types::PROTOCOL_VERSION
            );
            Ok(())
        }
        Command::RegisterBrowsers => anyhow::bail!("browser registration is not implemented yet"),
    }
}
