use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

pub fn find(name: &str) -> Option<std::path::PathBuf> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        let path = std::path::PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|entry| entry.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn run_status(program: &Path, args: &[OsString]) -> Result<ExitStatus> {
    Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch {}", program.display()))
}

pub fn run_checked(program: &Path, args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to launch {}", program.display()))?;
    if !status.success() {
        bail!("{} exited with {}", program.display(), exit_code(status));
    }
    Ok(())
}

pub fn output(program: &Path, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let result = command
        .output()
        .with_context(|| format!("failed to launch {}", program.display()))?;
    if !result.status.success() {
        bail!(
            "{} exited with {}: {}",
            program.display(),
            exit_code(result.status),
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

pub fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    128
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn signal_exit_code_uses_shell_convention() {
        let status = std::process::Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();
        assert_eq!(super::exit_code(status), 143);
    }
}
