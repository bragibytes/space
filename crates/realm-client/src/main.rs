mod app;
mod config;
mod plain;
mod tui;
mod ui;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "realm",
    about = "Realm of Echoes — a classic MMO text adventure",
    version,
    after_help = "Just run `realm` — it auto-connects to the live server.\nUse `realm --plain` if your terminal doesn't support full-screen mode."
)]
struct Args {
    /// Override WebSocket URL (skips auto-discovery)
    #[arg(long, env = "REALM_SERVER")]
    server: Option<String>,

    #[arg(long, env = "REALM_PLAIN")]
    plain: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let args = Args::parse();
    let server = config::resolve_server_url(args.server).await?;
    let use_plain = args.plain || !IsTerminal::is_terminal(&std::io::stdout());

    if use_plain {
        plain::run(&server).await
    } else {
        tui::run(&server).await
    }
}