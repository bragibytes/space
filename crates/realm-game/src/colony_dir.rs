use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const COLONY_DIR_NAME: &str = "colony";

/// Resolve `~/.creeps/colony` (override with `CREEPS_COLONY_DIR`).
pub fn colony_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CREEPS_COLONY_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().context("home directory")?;
    Ok(home.join(".creeps").join(COLONY_DIR_NAME))
}

/// Create the colony project from `templates/colony` if it does not exist.
pub fn ensure_colony_project() -> Result<PathBuf> {
    let dir = colony_dir()?;
    if dir.join("Cargo.toml").exists() {
        return Ok(dir);
    }

    fs::create_dir_all(&dir)?;
    let template = template_dir()?;
    copy_dir_recursive(&template, &dir)?;
    patch_sdk_dependency(&dir, &template)?;
    Ok(dir)
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