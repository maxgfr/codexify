use crate::caffeine::{self, Mode};
use crate::command;
use crate::paths::Paths;
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::{Command, Stdio};

pub fn run(paths: &Paths, args: &[OsString], one_shot: Option<Mode>) -> Result<i32> {
    maybe_auto_backup(paths);
    let codex = command::find("codex").context("codex was not found in PATH")?;
    let mode = one_shot.or(caffeine::configured(paths)?);
    let status = match mode {
        Some(mode) => caffeine::run(codex, args, mode)?,
        None => command::run_status(&codex, args)?,
    };
    Ok(command::exit_code(status))
}

fn maybe_auto_backup(paths: &Paths) {
    let Ok(state) = crate::state::ManagedState::load(paths) else {
        return;
    };
    if !state.backup_auto {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args(["backup", "push", "--quiet", "--timeout", "2"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
