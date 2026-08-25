use crate::command;
use crate::fsutil::{atomic_write, read_or_empty};
use crate::notify;
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use walkdir::WalkDir;

const BACKUP_HOOK_STATUS: &str = "Codexify config backup";
const HOME_TOKEN: &[u8] = b"$CODEXIFY_HOME$";
const ESCAPE_TOKEN: &[u8] = b"$CODEXIFY_ESCAPE$";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Repo,
    Gist,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RestoreMode {
    Mirror,
    Additive,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackupConfig {
    pub backend: Backend,
    pub target: String,
    #[serde(default = "default_mode")]
    pub mode: RestoreMode,
}

fn default_mode() -> RestoreMode {
    RestoreMode::Mirror
}

impl BackupConfig {
    pub fn load(paths: &Paths) -> Result<Self> {
        let raw = read_or_empty(&paths.backup_config())?;
        if raw.trim().is_empty() {
            bail!("backup is not configured; run 'codexify backup init repo|gist TARGET'");
        }
        toml::from_str(&raw).context("failed to parse backup.toml")
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        atomic_write(
            &paths.backup_config(),
            toml::to_string_pretty(self)?.as_bytes(),
            &paths.state,
        )
    }
}

pub fn init(paths: &Paths, backend: &str, target: &str, additive: bool) -> Result<()> {
    let backend = match backend {
        "repo" => Backend::Repo,
        "gist" => Backend::Gist,
        _ => bail!("backup backend must be 'repo' or 'gist'"),
    };
    if target.trim().is_empty() {
        bail!("backup target cannot be empty");
    }
    let target = if backend == Backend::Gist && target == "new" {
        create_secret_gist()?
    } else {
        target.to_owned()
    };
    let config = BackupConfig {
        backend,
        target,
        mode: if additive {
            RestoreMode::Additive
        } else {
            RestoreMode::Mirror
        },
    };
    config.save(paths)?;
    println!("Backup configured for {}.", config.target);
    Ok(())
}

pub fn status(paths: &Paths) -> Result<()> {
    let state = ManagedState::load(paths)?;
    match BackupConfig::load(paths) {
        Ok(config) => {
            println!("backend = {:?}", config.backend);
            println!("target = {}", config.target);
            println!("mode = {:?}", config.mode);
        }
        Err(_) => println!("not configured"),
    }
    println!("auto = {}", state.backup_auto);
    println!("hooks = {}", state.backup_hooks);
    Ok(())
}

pub fn set_auto(paths: &Paths, enabled: bool) -> Result<()> {
    if enabled {
        let _ = BackupConfig::load(paths)?;
    }
    let mut state = ManagedState::load(paths)?;
    state.backup_auto = enabled;
    state.save(paths)?;
    println!(
        "Automatic launch-time backup {}.",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

pub fn set_hooks(paths: &Paths, executable: &Path, enabled: bool) -> Result<()> {
    if enabled {
        let _ = BackupConfig::load(paths)?;
        install_backup_hook(paths, executable)?;
    } else {
        remove_backup_hook(paths)?;
    }
    let mut state = ManagedState::load(paths)?;
    state.backup_hooks = enabled;
    state.save(paths)?;
    println!(
        "Backup hook {}.",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

pub fn off(paths: &Paths) -> Result<()> {
    set_auto(paths, false)?;
    let executable = std::env::current_exe()?;
    set_hooks(paths, &executable, false)?;
    println!("Backup automation is off; remote data was not changed.");
    Ok(())
}

pub fn push(paths: &Paths, quiet: bool, timeout_seconds: Option<u64>) -> Result<()> {
    if let Some(seconds) = timeout_seconds {
        arm_timeout(seconds)?;
    }
    let config = BackupConfig::load(paths)?;
    let temp = tempfile::tempdir()?;
    let snapshot = temp.path().join("codex");
    collect_snapshot(paths, &snapshot)?;
    let findings = scan_secrets(&snapshot)?;
    if !findings.is_empty() {
        for finding in findings {
            eprintln!(
                "{}:{}: {}",
                finding.file.display(),
                finding.line,
                finding.kind
            );
        }
        bail!("backup blocked because potential secrets were detected");
    }
    match config.backend {
        Backend::Repo => push_repo(paths, &config, &snapshot)?,
        Backend::Gist => push_gist(&config, &snapshot)?,
    }
    if !quiet {
        println!("Backup pushed.");
    }
    Ok(())
}

pub fn pull(paths: &Paths, yes: bool, additive_override: bool) -> Result<()> {
    let mut config = BackupConfig::load(paths)?;
    if additive_override {
        config.mode = RestoreMode::Additive;
    }
    let temp = tempfile::tempdir()?;
    let snapshot = temp.path().join("codex");
    match config.backend {
        Backend::Repo => pull_repo(paths, &config, &snapshot)?,
        Backend::Gist => pull_gist(&config, &snapshot)?,
    }
    restore_snapshot(paths, &snapshot, config.mode, yes)?;
    println!("Backup restored. Local Codexify hooks were reattached when enabled.");
    Ok(())
}

pub fn collect_snapshot(paths: &Paths, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for relative in allowed_files(&paths.codex)? {
        let source = paths.codex.join(&relative);
        let destination = target.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = fs::read(&source)?;
        if relative == Path::new("config.toml") {
            bytes = portable_config(paths, &bytes)?;
        } else if relative == Path::new("hooks.json") {
            bytes = portable_hooks(paths, &bytes)?;
        } else {
            bytes = encode_portable(&bytes, paths.home.to_string_lossy().as_bytes());
        }
        fs::write(destination, bytes)?;
    }
    Ok(())
}

fn portable_config(paths: &Paths, bytes: &[u8]) -> Result<Vec<u8>> {
    let raw = String::from_utf8(bytes.to_vec()).context("config.toml is not UTF-8")?;
    let mut doc = raw.parse::<toml_edit::DocumentMut>()?;
    if doc
        .get("notify")
        .is_some_and(|item| item.to_string().contains("codexify"))
    {
        doc.remove("notify");
    }
    Ok(encode_portable(
        doc.to_string().as_bytes(),
        paths.home.to_string_lossy().as_bytes(),
    ))
}

fn portable_hooks(paths: &Paths, bytes: &[u8]) -> Result<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    strip_codexify_handlers(&mut value);
    let mut result = serde_json::to_vec_pretty(&value)?;
    result.push(b'\n');
    Ok(encode_portable(
        &result,
        paths.home.to_string_lossy().as_bytes(),
    ))
}

pub fn allowed_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = BTreeSet::new();
    for name in ["AGENTS.md", "config.toml", "hooks.json"] {
        if root.join(name).is_file() {
            result.insert(PathBuf::from(name));
        }
    }
    if root.is_dir() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            if entry.file_type()?.is_file() && name.to_string_lossy().ends_with(".config.toml") {
                result.insert(PathBuf::from(name));
            }
        }
    }
    for directory in ["hooks", "rules", "skills"] {
        let base = root.join(directory);
        if !base.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&base).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_file() {
                result.insert(entry.path().strip_prefix(root)?.to_path_buf());
            }
        }
    }
    Ok(result.into_iter().collect())
}

#[derive(Clone, Debug)]
pub struct SecretFinding {
    pub file: PathBuf,
    pub line: usize,
    pub kind: &'static str,
}

pub fn scan_secrets(root: &Path) -> Result<Vec<SecretFinding>> {
    let patterns = [
        (Regex::new(r"sk-[A-Za-z0-9_-]{20,}")?, "OpenAI API key"),
        (Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}")?, "GitHub token"),
        (Regex::new(r"AKIA[0-9A-Z]{16}")?, "AWS access key"),
        (
            Regex::new(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")?,
            "private key",
        ),
        (
            Regex::new(
                r#"(?i)(api[_-]?key|token|secret|password)\s*[=:]\s*['\"]?[A-Za-z0-9_./+=-]{16,}"#,
            )?,
            "credential assignment",
        ),
    ];
    let mut findings = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            for (regex, kind) in &patterns {
                if regex.is_match(line) {
                    findings.push(SecretFinding {
                        file: entry.path().strip_prefix(root)?.to_path_buf(),
                        line: index + 1,
                        kind,
                    });
                }
            }
        }
    }
    Ok(findings)
}

fn push_repo(paths: &Paths, config: &BackupConfig, snapshot: &Path) -> Result<()> {
    let work = prepare_repo(paths, &config.target)?;
    let destination = work.join("codex");
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    copy_tree(snapshot, &destination)?;
    let git = command::find("git").context("git was not found in PATH")?;
    command::run_checked(&git, &["add", "--all", "codex"], Some(&work))?;
    let changes = Command::new(&git)
        .args(["diff", "--cached", "--quiet"])
        .current_dir(&work)
        .status()?;
    if changes.success() {
        return Ok(());
    }
    command::run_checked(
        &git,
        &["commit", "-m", "chore: update Codex backup"],
        Some(&work),
    )?;
    command::run_checked(&git, &["push", "origin", "HEAD"], Some(&work))
}

fn pull_repo(paths: &Paths, config: &BackupConfig, snapshot: &Path) -> Result<()> {
    let work = prepare_repo(paths, &config.target)?;
    let source = work.join("codex");
    if !source.is_dir() {
        bail!("remote repository does not contain a codex/ snapshot");
    }
    copy_tree(&source, snapshot)
}

fn prepare_repo(paths: &Paths, target: &str) -> Result<PathBuf> {
    let work = paths.state.join("backup-repo");
    let git = command::find("git").context("git was not found in PATH")?;
    if work.join(".git").is_dir() {
        let remote = command::output(&git, &["remote", "get-url", "origin"], Some(&work))?;
        if remote == target {
            command::run_checked(&git, &["fetch", "origin"], Some(&work))?;
            command::run_checked(&git, &["pull", "--ff-only"], Some(&work))?;
            return Ok(work);
        }
        fs::remove_dir_all(&work)?;
    }
    if let Some(parent) = work.parent() {
        fs::create_dir_all(parent)?;
    }
    let work_string = work.to_string_lossy().into_owned();
    command::run_checked(&git, &["clone", target, &work_string], None)?;
    Ok(work)
}

fn push_gist(config: &BackupConfig, snapshot: &Path) -> Result<()> {
    let gh = command::find("gh").context("gh was not found in PATH")?;
    let mut files = Map::new();
    let mut manifest = BTreeMap::new();
    for entry in WalkDir::new(snapshot) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(snapshot)?
            .to_string_lossy()
            .replace('\\', "/");
        let flat = flatten_name(&relative);
        manifest.insert(flat.clone(), relative);
        files.insert(
            flat,
            json!({"content": hex_encode(&fs::read(entry.path())?)}),
        );
    }
    files.insert(
        "codexify-manifest.json".to_owned(),
        json!({"content": serde_json::to_string_pretty(&manifest)?}),
    );
    let existing = command::output(&gh, &["api", &format!("gists/{}", config.target)], None)?;
    let existing: Value = serde_json::from_str(&existing)?;
    if let Some(old_manifest) = existing
        .pointer("/files/codexify-manifest.json/content")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(raw).ok())
    {
        for old_name in old_manifest.keys() {
            if !files.contains_key(old_name) {
                files.insert(old_name.clone(), Value::Null);
            }
        }
    }
    let body = json!({"description": "Codex configuration backup by Codexify", "files": files});
    gh_api_input(
        &gh,
        &["--method", "PATCH", &format!("gists/{}", config.target)],
        &body,
    )?;
    Ok(())
}

fn pull_gist(config: &BackupConfig, snapshot: &Path) -> Result<()> {
    let gh = command::find("gh").context("gh was not found in PATH")?;
    let output = command::output(&gh, &["api", &format!("gists/{}", config.target)], None)?;
    let root: Value = serde_json::from_str(&output)?;
    let files = root
        .get("files")
        .and_then(Value::as_object)
        .context("gist has no files")?;
    let manifest_raw = files
        .get("codexify-manifest.json")
        .and_then(|file| file.get("content"))
        .and_then(Value::as_str)
        .context("gist does not contain codexify-manifest.json")?;
    let manifest: BTreeMap<String, String> = serde_json::from_str(manifest_raw)?;
    for (flat, relative) in manifest {
        let safe = safe_relative(&relative)?;
        let file = files
            .get(&flat)
            .with_context(|| format!("gist file '{flat}' is missing"))?;
        let content = hex_decode(&gist_file_content(file)?)?;
        let target = snapshot.join(safe);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, content)?;
    }
    Ok(())
}

fn create_secret_gist() -> Result<String> {
    let gh = command::find("gh").context("gh was not found in PATH")?;
    let mut placeholder = tempfile::NamedTempFile::new()?;
    placeholder.write_all(b"{}\n")?;
    let output = Command::new(gh)
        .args([
            "gist",
            "create",
            "--secret",
            "--desc",
            "Codex config backup",
        ])
        .arg(placeholder.path())
        .output()?;
    if !output.status.success() {
        bail!(
            "gh gist create exited with {}: {}",
            command::exit_code(output.status),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let url = String::from_utf8_lossy(&output.stdout);
    url.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .context("gh did not return a gist id")
}

fn gist_file_content(file: &Value) -> Result<String> {
    if !file
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return file
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("gist file has no content");
    }
    let url = file
        .get("raw_url")
        .and_then(Value::as_str)
        .context("truncated gist file has no raw_url")?;
    let curl = command::find("curl").context("curl is required for a truncated gist file")?;
    command::output(&curl, &["--fail", "--location", url], None)
}

fn gh_api_input(gh: &Path, args: &[&str], body: &Value) -> Result<()> {
    let mut child = Command::new(gh)
        .arg("api")
        .args(args)
        .arg("--input")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("failed to open gh stdin")?
        .write_all(serde_json::to_string(body)?.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("gh api exited with {}", command::exit_code(status));
    }
    Ok(())
}

fn restore_snapshot(paths: &Paths, snapshot: &Path, mode: RestoreMode, yes: bool) -> Result<()> {
    let remote: BTreeSet<_> = allowed_files(snapshot)?.into_iter().collect();
    let local: BTreeSet<_> = allowed_files(&paths.codex)?.into_iter().collect();
    let mut changes = Vec::new();
    for path in &remote {
        let source = fs::read(snapshot.join(path))?;
        let portable = decode_portable(&source, paths.home.to_string_lossy().as_bytes());
        let current = fs::read(paths.codex.join(path)).ok();
        if current.as_deref() != Some(&portable) {
            changes.push(format!(
                "{} {}",
                if current.is_some() { "M" } else { "A" },
                path.display()
            ));
        }
    }
    if mode == RestoreMode::Mirror {
        for path in local.difference(&remote) {
            changes.push(format!("D {}", path.display()));
        }
    }
    if changes.is_empty() {
        println!("Backup is already in sync.");
        return Ok(());
    }
    println!("Restore diff:");
    for change in &changes {
        println!("  {change}");
    }
    if !yes && !confirm()? {
        bail!("restore cancelled");
    }
    let state = ManagedState::load(paths)?;
    if state.notifications_enabled {
        notify::off(paths)?;
    }
    if state.backup_hooks {
        remove_backup_hook(paths)?;
    }
    for path in &remote {
        let bytes = fs::read(snapshot.join(path))?;
        let restored = decode_portable(&bytes, paths.home.to_string_lossy().as_bytes());
        atomic_write(&paths.codex.join(path), &restored, &paths.state)?;
    }
    if mode == RestoreMode::Mirror {
        for path in local.difference(&remote) {
            let target = paths.codex.join(path);
            crate::fsutil::backup_before_write(&target, &paths.state)?;
            fs::remove_file(target)?;
        }
    }
    let executable = std::env::current_exe()?;
    if state.notifications_enabled {
        notify::on(paths, &executable)?;
    }
    if state.backup_hooks {
        install_backup_hook(paths, &executable)?;
    }
    Ok(())
}

fn confirm() -> Result<bool> {
    print!("Apply this restore? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn flatten_name(relative: &str) -> String {
    format!("codexify-{}", hex_encode(relative.as_bytes()))
}

fn encode_portable(input: &[u8], home: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index..].starts_with(ESCAPE_TOKEN) {
            output.extend_from_slice(ESCAPE_TOKEN);
            output.push(b'E');
            index += ESCAPE_TOKEN.len();
        } else if input[index..].starts_with(HOME_TOKEN) {
            output.extend_from_slice(ESCAPE_TOKEN);
            output.push(b'H');
            index += HOME_TOKEN.len();
        } else if !home.is_empty() && input[index..].starts_with(home) {
            output.extend_from_slice(HOME_TOKEN);
            index += home.len();
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    output
}

fn decode_portable(input: &[u8], home: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index..].starts_with(ESCAPE_TOKEN)
            && input.get(index + ESCAPE_TOKEN.len()) == Some(&b'E')
        {
            output.extend_from_slice(ESCAPE_TOKEN);
            index += ESCAPE_TOKEN.len() + 1;
        } else if input[index..].starts_with(ESCAPE_TOKEN)
            && input.get(index + ESCAPE_TOKEN.len()) == Some(&b'H')
        {
            output.extend_from_slice(HOME_TOKEN);
            index += ESCAPE_TOKEN.len() + 1;
        } else if input[index..].starts_with(HOME_TOKEN) {
            output.extend_from_slice(home);
            index += HOME_TOKEN.len();
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    output
}

fn hex_encode(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(input: &str) -> Result<Vec<u8>> {
    if input.len() % 2 != 0 {
        bail!("invalid hex-encoded gist content");
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .context("invalid hex-encoded gist content")?;
            let low = (pair[1] as char)
                .to_digit(16)
                .context("invalid hex-encoded gist content")?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn arm_timeout(seconds: u64) -> Result<()> {
    if seconds == 0 {
        bail!("backup timeout must be at least one second");
    }
    #[cfg(unix)]
    {
        // Isolate the backup and every subprocess it spawns so the watchdog can
        // terminate the entire bounded operation without touching Codex.
        let result = unsafe { libc::setpgid(0, 0) };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to isolate backup process");
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            unsafe {
                libc::kill(0, libc::SIGKILL);
            }
        });
    }
    #[cfg(not(unix))]
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        std::process::exit(124);
    });
    Ok(())
}

fn safe_relative(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("unsafe path in backup manifest: {value}");
    }
    Ok(path)
}

fn strip_codexify_handlers(root: &mut Value) {
    let Some(events) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for groups in events.values_mut().filter_map(Value::as_array_mut) {
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|handler| {
                    !handler
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|command| command.contains("codexify "))
                });
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|handlers| !handlers.is_empty())
        });
    }
}

fn install_backup_hook(paths: &Paths, executable: &Path) -> Result<()> {
    let raw = read_or_empty(&paths.hooks())?;
    let mut root: Value = if raw.trim().is_empty() {
        json!({"hooks": {}})
    } else {
        serde_json::from_str(&raw)?
    };
    remove_backup_handler(&mut root);
    let events = root
        .as_object_mut()
        .context("hooks.json root must be an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let stop = events
        .as_object_mut()
        .context("hooks must be an object")?
        .entry("Stop")
        .or_insert_with(|| json!([]));
    let executable_string = executable.to_string_lossy();
    let quoted = shell_words::quote(executable_string.as_ref());
    stop.as_array_mut()
        .context("hooks.Stop must be an array")?
        .push(json!({
            "hooks": [{
                "type": "command",
            "command": format!("{quoted} backup push --quiet --timeout 5"),
                "async": true,
                "timeout": 5,
                "statusMessage": BACKUP_HOOK_STATUS
            }]
        }));
    write_hooks(paths, &root)
}

fn remove_backup_hook(paths: &Paths) -> Result<()> {
    let raw = read_or_empty(&paths.hooks())?;
    if raw.trim().is_empty() {
        return Ok(());
    }
    let mut root: Value = serde_json::from_str(&raw)?;
    remove_backup_handler(&mut root);
    write_hooks(paths, &root)
}

fn remove_backup_handler(root: &mut Value) {
    let Some(groups) = root
        .pointer_mut("/hooks/Stop")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for group in groups.iter_mut() {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            handlers.retain(|handler| {
                handler.get("statusMessage").and_then(Value::as_str) != Some(BACKUP_HOOK_STATUS)
            });
        }
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|handlers| !handlers.is_empty())
    });
}

fn write_hooks(paths: &Paths, root: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(root)?;
    bytes.push(b'\n');
    atomic_write(&paths.hooks(), &bytes, &paths.state)
}

#[cfg(test)]
mod tests {
    use super::{
        allowed_files, decode_portable, encode_portable, flatten_name, hex_decode, hex_encode,
        scan_secrets,
    };
    use std::fs;

    #[test]
    fn allowlist_excludes_sensitive_runtime_data() {
        let temp = tempfile::tempdir().unwrap();
        for name in [
            "AGENTS.md",
            "config.toml",
            "hooks.json",
            "work.config.toml",
            "auth.json",
            "history.jsonl",
            "state.sqlite",
        ] {
            fs::write(temp.path().join(name), "x").unwrap();
        }
        fs::create_dir(temp.path().join("skills")).unwrap();
        fs::write(temp.path().join("skills/demo.md"), "ok").unwrap();
        let files = allowed_files(temp.path()).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "AGENTS.md",
                "config.toml",
                "hooks.json",
                "skills/demo.md",
                "work.config.toml"
            ]
        );
    }

    #[test]
    fn secret_scan_reports_location_and_type() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "token = \"ghp_abcdefghijklmnopqrstuvwxyz123456\"\n",
        )
        .unwrap();
        let findings = scan_secrets(temp.path()).unwrap();
        assert!(
            findings
                .iter()
                .any(|finding| finding.line == 1 && finding.kind == "GitHub token")
        );
    }

    #[test]
    fn portability_transform_is_binary_safe_and_escapes_literal_tokens() {
        let home = b"/home/tester";
        let input = b"\xff/home/tester/bin $CODEXIFY_HOME$ $CODEXIFY_ESCAPE$E\x00";
        let encoded = encode_portable(input, home);
        assert!(
            encoded
                .windows(b"$CODEXIFY_HOME$".len())
                .any(|v| v == b"$CODEXIFY_HOME$")
        );
        assert_eq!(decode_portable(&encoded, home), input);
    }

    #[test]
    fn gist_names_and_contents_are_injective_and_binary_safe() {
        assert_ne!(flatten_name("skills/a_/b"), flatten_name("skills/a/_b"));
        let bytes = b"\x00\xffbinary";
        assert_eq!(hex_decode(&hex_encode(bytes)).unwrap(), bytes);
    }
}
