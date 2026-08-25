use anyhow::{Context, Result};
use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn read_or_empty(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub fn backup_before_write(path: &Path, state_dir: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup_dir = state_dir.join("backups");
    fs::create_dir_all(&backup_dir)
        .with_context(|| format!("failed to create {}", backup_dir.display()))?;
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("file");
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.6fZ");
    let target = backup_dir.join(format!("{name}.{stamp}.bak"));
    fs::copy(path, &target).with_context(|| {
        format!(
            "failed to back up {} to {}",
            path.display(),
            target.display()
        )
    })?;
    Ok(Some(target))
}

pub fn atomic_write(path: &Path, bytes: &[u8], state_dir: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if path.exists() && fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    backup_before_write(path, state_dir)?;
    let parent = path.parent().context("target has no parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    temp.as_file_mut().write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {} atomically", path.display()))?;
    sync_parent(parent)?;
    Ok(())
}

fn sync_parent(parent: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

pub fn copy_file_atomic(source: &Path, target: &Path, state_dir: &Path) -> Result<()> {
    let bytes = fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
    atomic_write(target, &bytes, state_dir)
}
