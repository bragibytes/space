use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Default player source directory: `~/.creeps/colony/`.
pub fn default_source_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory")?;
    Ok(home.join(".creeps").join("colony"))
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("config directory")?
        .join("creeps");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn source_config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("source-dir.txt"))
}

/// Last path chosen in the UI (persisted across sessions).
pub fn load_saved_source_dir() -> Option<PathBuf> {
    let path = source_config_file().ok()?;
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

pub fn save_source_dir(dir: &Path) -> Result<()> {
    let file = source_config_file()?;
    fs::write(file, dir.to_string_lossy().as_bytes())?;
    Ok(())
}

/// Resolve source dir: env `CREEPS_COLONY_DIR` → `REALM_SOURCE_DIR` → saved UI path → `~/.creeps/colony/`.
pub fn resolve_source_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CREEPS_COLONY_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("REALM_SOURCE_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = load_saved_source_dir() {
        return Ok(path);
    }
    default_source_dir()
}

/// Create a starter Rust project at `dir` if `Cargo.toml` is missing.
pub fn ensure_colony_project_at(dir: &Path) -> Result<PathBuf> {
    if dir.join("Cargo.toml").exists() {
        return Ok(dir.to_path_buf());
    }

    fs::create_dir_all(dir)?;
    let template = template_dir()?;
    copy_dir_recursive(&template, dir)?;
    patch_sdk_dependency(dir, &template)?;
    Ok(dir.to_path_buf())
}

/// Back-compat wrapper used at startup.
pub fn ensure_colony_project() -> Result<PathBuf> {
    let dir = resolve_source_dir()?;
    ensure_colony_project_at(&dir)
}

/// Prefer a path dependency when developing from the creeps repo checkout.
fn patch_sdk_dependency(colony_dir: &Path, template_dir: &Path) -> Result<()> {
    let sdk = template_dir
        .join("../../crates/realm-sdk")
        .canonicalize()
        .ok()
        .filter(|p| p.join("Cargo.toml").exists());
    let Some(sdk) = sdk else {
        return Ok(());
    };
    let cargo_toml = colony_dir.join("Cargo.toml");
    let mut contents = fs::read_to_string(&cargo_toml)?;
    if contents.contains("git = ") {
        contents = contents.replace(
            r#"realm-sdk = { git = "https://github.com/bragibytes/creeps" }"#,
            &format!(r#"realm-sdk = {{ path = "{}" }}"#, sdk.display()),
        );
        fs::write(cargo_toml, contents)?;
    }
    Ok(())
}

fn template_dir() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let from_repo = manifest
        .join("../../templates/colony")
        .canonicalize()
        .ok();
    if let Some(path) = from_repo {
        if path.join("Cargo.toml").exists() {
            return Ok(path);
        }
    }
    anyhow::bail!("templates/colony not found (run from the creeps repo checkout)")
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            fs::create_dir_all(&to)?;
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn wasm_output_path(colony_dir: &Path) -> PathBuf {
    colony_dir.join("target/wasm32-unknown-unknown/release/colony.wasm")
}

pub fn expand_path(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}