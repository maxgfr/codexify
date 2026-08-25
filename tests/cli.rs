#![cfg(unix)]

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

struct Sandbox {
    temp: tempfile::TempDir,
    codex: PathBuf,
    state: PathBuf,
    bin: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join("codex");
        let state = temp.path().join("codexify");
        let bin = temp.path().join("bin");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&bin).unwrap();
        Self {
            temp,
            codex,
            state,
            bin,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("codexify").unwrap();
        command
            .env("CODEX_HOME", &self.codex)
            .env("CODEXIFY_STATE_DIR", &self.state)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        command
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

#[test]
fn model_selection_preserves_unknown_toml_and_reset_restores_native_state() {
    // FR-001 model/profile selection and native reset.
    let sandbox = Sandbox::new();
    let original = "# user comment\ncustom_key = \"keep\"\n\n[custom]\nenabled = true\n";
    fs::write(sandbox.codex.join("config.toml"), original).unwrap();

    sandbox
        .command()
        .args(["use", "quality"])
        .assert()
        .success();
    let selected = fs::read_to_string(sandbox.codex.join("config.toml")).unwrap();
    assert!(selected.contains("# user comment"));
    assert!(selected.contains("custom_key = \"keep\""));
    assert!(selected.contains("model = \"gpt-5.6\""));
    assert!(selected.contains("model_reasoning_effort = \"xhigh\""));

    sandbox.command().arg("reset").assert().success();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        original
    );
    assert!(sandbox.state.join("backups").is_dir());
}

#[test]
fn profile_use_requires_an_existing_profile() {
    // FR-001 profile semantics distinguish names from raw model selectors.
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["profile", "use", "does-not-exist"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown profile"));
    assert!(!sandbox.codex.join("config.toml").exists());
}

#[test]
fn notification_on_repairs_action_required_and_is_reversible_and_idempotent() {
    // FR-002 reliable, reversible Action Required notifications.
    let sandbox = Sandbox::new();
    let original_config = include_str!("fixtures/action-required-bug.toml");
    let original_hooks: Value =
        serde_json::from_str(include_str!("fixtures/existing-hooks.json")).unwrap();
    fs::write(sandbox.codex.join("config.toml"), original_config).unwrap();
    fs::write(
        sandbox.codex.join("hooks.json"),
        serde_json::to_vec_pretty(&original_hooks).unwrap(),
    )
    .unwrap();

    sandbox.command().args(["notify", "on"]).assert().success();
    sandbox.command().args(["notify", "on"]).assert().success();

    let config = fs::read_to_string(sandbox.codex.join("config.toml")).unwrap();
    let doc = config.parse::<toml_edit::DocumentMut>().unwrap();
    let callback = doc["notify"].as_array().unwrap();
    let binary = callback.get(0).and_then(|value| value.as_str()).unwrap();
    assert!(Path::new(binary).is_absolute());
    assert!(!doc["notify"].to_string().contains('~'));
    let events: Vec<_> = doc["tui"]["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(
        events,
        ["agent-turn-complete", "custom-event", "approval-requested"]
    );
    assert_eq!(doc["tui"]["notification_method"].as_str(), Some("bel"));
    assert_eq!(
        doc["tui"]["notification_condition"].as_str(),
        Some("always")
    );

    let hooks: Value =
        serde_json::from_slice(&fs::read(sandbox.codex.join("hooks.json")).unwrap()).unwrap();
    let permission = hooks
        .pointer("/hooks/PermissionRequest")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        permission.len(),
        2,
        "second notify on must not duplicate the managed hook"
    );
    let managed = permission
        .iter()
        .flat_map(|group| group["hooks"].as_array().unwrap())
        .find(|handler| handler["statusMessage"] == "Codexify Action Required notification")
        .unwrap();
    assert_eq!(managed["async"], true);
    assert!(
        managed["command"]
            .as_str()
            .unwrap()
            .contains(" notify emit --source hook")
    );
    assert!(
        managed.get("decision").is_none(),
        "notification hooks must not decide approvals"
    );

    sandbox.command().args(["notify", "off"]).assert().success();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        original_config
    );
    let restored: Value =
        serde_json::from_slice(&fs::read(sandbox.codex.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(restored, original_hooks);
}

#[test]
fn notification_on_preserves_all_events_when_notifications_are_true() {
    let sandbox = Sandbox::new();
    let original = "[tui]\nnotifications = true\n";
    fs::write(sandbox.codex.join("config.toml"), original).unwrap();

    sandbox.command().args(["notify", "on"]).assert().success();

    let config = fs::read_to_string(sandbox.codex.join("config.toml")).unwrap();
    let doc = config.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["tui"]["notifications"].as_bool(), Some(true));

    sandbox.command().args(["notify", "off"]).assert().success();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        original
    );
}

#[test]
fn failed_notification_install_remains_recoverable_by_purge() {
    // FR-002 a cross-file failure cannot orphan the previous callback.
    let sandbox = Sandbox::new();
    let original = include_str!("fixtures/action-required-bug.toml");
    fs::write(sandbox.codex.join("config.toml"), original).unwrap();
    fs::write(sandbox.codex.join("hooks.json"), "{\"hooks\": []}\n").unwrap();
    sandbox.command().args(["notify", "on"]).assert().failure();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        original
    );
    assert!(!sandbox.state.join("state.json").exists());
    fs::write(sandbox.codex.join("hooks.json"), "{\"hooks\": {}}\n").unwrap();
    sandbox.command().arg("purge").assert().success();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        original
    );
    assert!(!sandbox.state.exists());
}

#[test]
fn existing_callback_runs_even_when_codexify_delivery_fails() {
    // FR-002 preserve existing desktop callbacks across backend failures.
    let sandbox = Sandbox::new();
    let marker = sandbox.temp.path().join("callback-ran");
    let callback = sandbox.script(
        "desktop-callback",
        &format!("printf '%s' \"$1\" > '{}'", marker.display()),
    );
    fs::write(
        sandbox.codex.join("config.toml"),
        format!("notify = [\"{}\"]\n", callback.display()),
    )
    .unwrap();
    sandbox.command().args(["notify", "on"]).assert().success();
    sandbox
        .command()
        .env("PATH", "/path/that/does/not/exist")
        .env_remove("TERM_PROGRAM")
        .env_remove("KITTY_WINDOW_ID")
        .args([
            "notify",
            "emit",
            "--source",
            "codex",
            r#"{"type":"agent-turn-complete","last-assistant-message":"done"}"#,
        ])
        .assert()
        .failure();
    for _ in 0..50 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists());
    assert!(
        fs::read_to_string(marker)
            .unwrap()
            .contains("agent-turn-complete")
    );
}

#[test]
fn codex_arguments_and_exit_code_are_forwarded_exactly() {
    // FR-003 exact Codex delegation.
    let sandbox = Sandbox::new();
    let output = sandbox.temp.path().join("codex-args");
    sandbox.script(
        "codex",
        &format!("printf '%s\\n' \"$@\" > '{}'\nexit 23", output.display()),
    );
    sandbox
        .command()
        .args(["exec", "--model", "gpt-test", "prompt with spaces"])
        .assert()
        .code(23);
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "exec\n--model\ngpt-test\nprompt with spaces\n"
    );
}

#[test]
fn conforme_arguments_and_exit_code_are_forwarded_exactly() {
    // FR-004 exact Conforme delegation.
    let sandbox = Sandbox::new();
    let output = sandbox.temp.path().join("conforme-args");
    sandbox.script(
        "conforme",
        &format!("printf '%s\\n' \"$@\" > '{}'\nexit 37", output.display()),
    );
    sandbox
        .command()
        .args(["sync", "--dry-run", "--only", "cursor,codex"])
        .assert()
        .code(37);
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "--dry-run\n--only\ncursor,codex\n"
    );
}

#[test]
fn caffeine_lock_wraps_the_child_and_releases_after_failure() {
    // FR-005 keep-awake child lifetime.
    let sandbox = Sandbox::new();
    let lock = sandbox.temp.path().join("awake.lock");
    let observed = sandbox.temp.path().join("observed");
    sandbox.script(
        "codex",
        &format!(
            "test -f '{}'\nprintf '%s\\n' \"$@\" > '{}'\nexit 19",
            lock.display(),
            observed.display()
        ),
    );
    if cfg!(target_os = "macos") {
        sandbox.script(
            "caffeinate",
            &format!(
                "shift\ntouch '{}'\n\"$@\" || code=$?\nrm -f '{}'\nexit ${{code:-0}}",
                lock.display(),
                lock.display()
            ),
        );
    } else {
        sandbox.script(
            "systemd-inhibit",
            &format!(
                "while [ \"${{1#--}}\" != \"$1\" ]; do shift; done\ntouch '{}'\n\"$@\" || code=$?\nrm -f '{}'\nexit ${{code:-0}}",
                lock.display(),
                lock.display()
            ),
        );
    }
    sandbox
        .command()
        .args(["--caffeine=system", "exec", "hello"])
        .assert()
        .code(19);
    assert!(!lock.exists());
    assert_eq!(fs::read_to_string(observed).unwrap(), "exec\nhello\n");
}

#[test]
fn git_backup_uses_allowlist_blocks_secrets_and_restores_in_mirror_mode() {
    // FR-006 allow-listed, secret-gated, reversible backup.
    let sandbox = Sandbox::new();
    let remote = sandbox.temp.path().join("remote.git");
    std::process::Command::new("git")
        .args(["init", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap();
    fs::write(sandbox.codex.join("config.toml"), "model = \"original\"\n").unwrap();
    fs::write(sandbox.codex.join("AGENTS.md"), "# Agents\n").unwrap();
    fs::write(sandbox.codex.join("auth.json"), "never upload").unwrap();
    fs::create_dir(sandbox.codex.join("skills")).unwrap();
    fs::write(sandbox.codex.join("skills/keep.md"), "keep").unwrap();
    fs::write(sandbox.codex.join("skills/binary.bin"), [0xff, 0x00, 0x80]).unwrap();

    sandbox
        .command()
        .args(["backup", "init", "repo", remote.to_str().unwrap()])
        .assert()
        .success();
    sandbox
        .command()
        .env("GIT_AUTHOR_NAME", "Codexify Test")
        .env("GIT_AUTHOR_EMAIL", "codexify@example.invalid")
        .env("GIT_COMMITTER_NAME", "Codexify Test")
        .env("GIT_COMMITTER_EMAIL", "codexify@example.invalid")
        .args(["backup", "push"])
        .assert()
        .success();

    fs::write(sandbox.codex.join("config.toml"), "model = \"changed\"\n").unwrap();
    fs::write(sandbox.codex.join("skills/local-only.md"), "delete me").unwrap();
    sandbox
        .command()
        .args(["backup", "pull", "--yes", "--additive"])
        .assert()
        .success();
    assert!(sandbox.codex.join("skills/local-only.md").exists());
    sandbox
        .command()
        .args(["backup", "pull", "--yes"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        "model = \"original\"\n"
    );
    assert!(!sandbox.codex.join("skills/local-only.md").exists());
    assert_eq!(
        fs::read(sandbox.codex.join("skills/binary.bin")).unwrap(),
        [0xff, 0x00, 0x80]
    );
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("auth.json")).unwrap(),
        "never upload"
    );

    fs::write(
        sandbox.codex.join("config.toml"),
        "token = \"ghp_abcdefghijklmnopqrstuvwxyz123456\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["backup", "push"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("config.toml:1: GitHub token"))
        .stderr(predicates::str::contains("ghp_").not());
}

#[test]
fn backup_timeout_is_enforced_inside_the_backup_process() {
    // FR-006 bounded automation survives a fast parent exit.
    let sandbox = Sandbox::new();
    fs::write(sandbox.codex.join("config.toml"), "model = \"test\"\n").unwrap();
    sandbox.script("git", "sleep 10");
    sandbox
        .command()
        .args(["backup", "init", "repo", "example.invalid/repo"])
        .assert()
        .success();
    let started = std::time::Instant::now();
    sandbox
        .command()
        .args(["backup", "push", "--quiet", "--timeout", "1"])
        .timeout(std::time::Duration::from_secs(4))
        .assert()
        .failure();
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

#[test]
fn divergent_backup_cache_fails_instead_of_restoring_stale_data() {
    // FR-006 repository conflicts fail closed.
    let sandbox = Sandbox::new();
    let remote = sandbox.temp.path().join("conflict-remote.git");
    run_git(
        sandbox.temp.path(),
        &["init", "--bare", remote.to_str().unwrap()],
    );
    fs::write(sandbox.codex.join("config.toml"), "model = \"base\"\n").unwrap();
    sandbox
        .command()
        .args(["backup", "init", "repo", remote.to_str().unwrap()])
        .assert()
        .success();
    sandbox
        .command()
        .env("GIT_AUTHOR_NAME", "Codexify Test")
        .env("GIT_AUTHOR_EMAIL", "codexify@example.invalid")
        .env("GIT_COMMITTER_NAME", "Codexify Test")
        .env("GIT_COMMITTER_EMAIL", "codexify@example.invalid")
        .args(["backup", "push"])
        .assert()
        .success();

    let cache = sandbox.state.join("backup-repo");
    fs::write(cache.join("codex/config.toml"), "model = \"stale\"\n").unwrap();
    run_git(&cache, &["add", "codex/config.toml"]);
    run_git(
        &cache,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "stale",
        ],
    );

    let remote_work = sandbox.temp.path().join("remote-work");
    run_git(
        sandbox.temp.path(),
        &[
            "clone",
            remote.to_str().unwrap(),
            remote_work.to_str().unwrap(),
        ],
    );
    fs::write(
        remote_work.join("codex/config.toml"),
        "model = \"remote-v2\"\n",
    )
    .unwrap();
    run_git(&remote_work, &["add", "codex/config.toml"]);
    run_git(
        &remote_work,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "remote",
        ],
    );
    run_git(&remote_work, &["push", "origin", "HEAD"]);

    fs::write(
        sandbox.codex.join("config.toml"),
        "model = \"safe-local\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["backup", "pull", "--yes"])
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(sandbox.codex.join("config.toml")).unwrap(),
        "model = \"safe-local\"\n"
    );
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}
