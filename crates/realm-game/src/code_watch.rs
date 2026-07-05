use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::colony_dir::wasm_output_path;

#[derive(Debug, Clone)]
pub enum CodeEvent {
    Compiling,
    Compiled {
        wasm: Vec<u8>,
    },
    CompileFailed {
        error: String,
    },
}

/// Watch `colony_dir` and send build events on a channel (poll from the main thread).
pub fn start_watching(colony_dir: PathBuf) -> Result<Receiver<CodeEvent>> {
    let (tx, rx) = mpsc::channel();
    spawn_watcher(colony_dir.clone(), tx.clone())?;
    thread::spawn(move || {
        let _ = run_build(&colony_dir, &tx);
    });
    Ok(rx)
}

fn spawn_watcher(colony_dir: PathBuf, tx: Sender<CodeEvent>) -> Result<()> {
    let (notify_tx, notify_rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(notify_tx, Config::default())?;
    watcher.watch(&colony_dir, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        let _watcher = watcher;
        let mut pending: Option<Instant> = None;
        loop {
            match notify_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(event)) if is_source_event(&event) => {
                    pending = Some(Instant::now());
                }
                Ok(Err(_)) | Err(_) => {}
                Ok(Ok(_)) => {}
            }

            if pending.is_some_and(|t| t.elapsed() >= Duration::from_millis(400)) {
                pending = None;
                let _ = run_build(&colony_dir, &tx);
            }
        }
    });

    Ok(())
}

fn is_source_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    ) && event.paths.iter().any(|p| {
        p.extension().is_some_and(|e| e == "rs" || e == "toml")
    })
}

fn run_build(colony_dir: &Path, tx: &Sender<CodeEvent>) -> Result<()> {
    let _ = tx.send(CodeEvent::Compiling);

    let output = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
            colony_dir.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .context("spawn cargo")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = tx.send(CodeEvent::CompileFailed {
            error: stderr.trim().to_string(),
        });
        return Ok(());
    }

    let wasm_path = wasm_output_path(colony_dir);
    let wasm = std::fs::read(&wasm_path)
        .with_context(|| format!("read {}", wasm_path.display()))?;
    let _ = tx.send(CodeEvent::Compiled { wasm });
    Ok(())
}