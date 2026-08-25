use crate::command;
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Mode {
    System,
    Display,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "display" => Ok(Self::Display),
            _ => bail!("invalid caffeine mode '{value}' (expected system or display)"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Display => "display",
        }
    }
}

pub fn set(paths: &Paths, mode: Option<Mode>) -> Result<()> {
    let mut state = ManagedState::load(paths)?;
    state.caffeine_mode = mode.map(|value| value.as_str().to_owned());
    state.save(paths)?;
    match mode {
        Some(value) => println!("Caffeine enabled ({})", value.as_str()),
        None => println!("Caffeine disabled"),
    }
    Ok(())
}

pub fn configured(paths: &Paths) -> Result<Option<Mode>> {
    ManagedState::load(paths)?
        .caffeine_mode
        .as_deref()
        .map(Mode::parse)
        .transpose()
}

pub fn status(paths: &Paths) -> Result<()> {
    match configured(paths)? {
        Some(mode) => println!("on ({})", mode.as_str()),
        None => println!("off"),
    }
    Ok(())
}

pub fn run(program: PathBuf, args: &[OsString], mode: Mode) -> Result<ExitStatus> {
    #[cfg(target_os = "macos")]
    {
        let caffeinate = command::find("caffeinate")
            .ok_or_else(|| anyhow::anyhow!("caffeinate is not available"))?;
        let flag = match mode {
            Mode::System => "-i",
            Mode::Display => "-d",
        };
        Command::new(caffeinate)
            .arg(flag)
            .arg(program)
            .args(args)
            .status()
            .map_err(Into::into)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(inhibit) = command::find("systemd-inhibit") {
            let what = match mode {
                Mode::System => "sleep",
                Mode::Display => "idle",
            };
            return Command::new(inhibit)
                .arg(format!("--what={what}"))
                .arg("--why=codexify session")
                .arg("--mode=block")
                .arg(program)
                .args(args)
                .status()
                .map_err(Into::into);
        }
        eprintln!("warning: no supported keep-awake backend; launching Codex normally");
        command::run_status(&program, args)
    }
}
