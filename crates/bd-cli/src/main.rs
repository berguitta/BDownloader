//! `bdl`: download links from the command line with the BDownloader engine.
//!
//! ```text
//! bdl get https://example.com/file.zip -o D:\Downloads -c 8
//! bdl probe https://example.com/file.zip
//! ```
//!
//! Press Ctrl+C to stop: progress is saved and the same command resumes.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bd_core::Settings;
use bd_net::{DownloadRequest, Engine};
use clap::{Parser, Subcommand};
use url::Url;

#[derive(Parser)]
#[command(name = "bdl", version, about = "BDownloader command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download one or more links, one after the other.
    Get {
        urls: Vec<Url>,
        /// Destination folder (defaults to the one in the app settings).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Parallel connections per file.
        #[arg(short, long, default_value_t = 8)]
        connections: usize,
        /// Ignore saved progress and start over.
        #[arg(long)]
        no_resume: bool,
    },
    /// Show what the server says about a link without downloading it.
    Probe { url: Url },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("BD_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let engine = Engine::new()?;

    match cli.command {
        Command::Probe { url } => {
            let remote = engine.probe(&url).await?;
            println!("URL:          {}", remote.url);
            println!("File name:    {}", remote.file_name.as_deref().unwrap_or("-"));
            println!("Size:         {}", remote.size.map(format_bytes).unwrap_or_else(|| "unknown".into()));
            println!("Ranges:       {}", if remote.ranges { "yes (multi-connection and resume)" } else { "no" });
            println!("ETag:         {}", remote.etag.as_deref().unwrap_or("-"));
            println!("Last-Modified {}", remote.last_modified.as_deref().unwrap_or("-"));
        }
        Command::Get { urls, output, connections, no_resume } => {
            if urls.is_empty() {
                bail!("give at least one link");
            }
            let dir = match output {
                Some(dir) => dir,
                None => Settings::default_path().map(|p| Settings::load_from(&p)).unwrap_or_default().download_dir,
            };
            for url in urls {
                let request = DownloadRequest::new(url.clone(), &dir).connections(connections).resume(!no_resume);
                download(&engine, request).await.with_context(|| format!("downloading {url}"))?;
            }
        }
    }
    Ok(())
}

async fn download(engine: &Engine, request: DownloadRequest) -> Result<()> {
    let task = engine.start(request);
    let control = task.control();
    let started = Instant::now();
    let mut last = (Instant::now(), 0u64);
    let mut speed = 0.0f64;
    let mut first = true;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    let mut wait = std::pin::pin!(task.wait());

    let result = loop {
        tokio::select! {
            result = &mut wait => break result,
            _ = tokio::signal::ctrl_c() => {
                control.cancel();
                eprintln!("\nStopping, progress is saved. Run the same command to resume.");
            }
            _ = ticker.tick() => {
                let p = control.progress();
                let now = Instant::now();
                if first && p.total.is_some() {
                    // Do not count bytes from a previous run as speed.
                    last = (now, p.downloaded);
                    first = false;
                }
                let dt = now.duration_since(last.0).as_secs_f64();
                if dt > 0.0 {
                    let instant = p.downloaded.saturating_sub(last.1) as f64 / dt;
                    speed = if speed == 0.0 { instant } else { speed * 0.7 + instant * 0.3 };
                    last = (now, p.downloaded);
                }
                let name = p.file_name.as_deref().unwrap_or("…");
                let line = match p.total {
                    Some(total) if total > 0 => format!(
                        "{name}  {:5.1}%  {} / {}  {}/s  {} conn{}",
                        p.downloaded as f64 * 100.0 / total as f64,
                        format_bytes(p.downloaded),
                        format_bytes(total),
                        format_bytes(speed as u64),
                        p.active_connections,
                        if p.resumed { "  (resumed)" } else { "" }
                    ),
                    _ => format!("{name}  {}  {}/s", format_bytes(p.downloaded), format_bytes(speed as u64)),
                };
                eprint!("\r{line:<100}");
                let _ = std::io::stderr().flush();
            }
        }
    };
    eprintln!();
    let path = result?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let secs = started.elapsed().as_secs_f64();
    println!(
        "Saved {} ({}) in {:.1} s, average {}/s",
        path.display(),
        format_bytes(size),
        secs,
        format_bytes((size as f64 / secs.max(0.001)) as u64)
    );
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
