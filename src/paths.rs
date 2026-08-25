use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub codex: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let home = dirs::home_dir().context("could not determine the home directory")?;
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let state = std::env::var_os("CODEXIFY_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codexify"));
        Ok(Self { home, codex, state })
    }

    pub fn config(&self) -> PathBuf {
        self.codex.join("config.toml")
    }

    pub fn hooks(&self) -> PathBuf {
        self.codex.join("hooks.json")
    }

    pub fn profiles(&self) -> PathBuf {
        self.state.join("profiles.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state.join("state.json")
    }

    pub fn backup_config(&self) -> PathBuf {
        self.state.join("backup.toml")
    }
}
