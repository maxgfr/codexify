use crate::backup::BackupConfig;
use crate::caffeine;
use crate::codex_config;
use crate::command;
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::Result;
use toml_edit::Item;

pub fn doctor(paths: &Paths) -> Result<i32> {
    let mut failures = 0;
    check(
        "codex in PATH",
        command::find("codex").is_some(),
        &mut failures,
    );
    check(
        "conforme in PATH",
        command::find("conforme").is_some(),
        &mut failures,
    );
    check(
        "Codex config parses",
        codex_config::load(paths).is_ok(),
        &mut failures,
    );

    let doc = codex_config::load(paths).ok();
    let notify = doc
        .as_ref()
        .and_then(|doc| doc.get("notify"))
        .and_then(Item::as_array);
    let callback_ok = notify
        .and_then(|array| array.get(0))
        .and_then(|value| value.as_str())
        .is_none_or(|value| std::path::Path::new(value).is_absolute() && !value.contains('~'));
    check(
        "notification callback path is portable",
        callback_ok,
        &mut failures,
    );

    let caffeine_ok = if cfg!(target_os = "macos") {
        command::find("caffeinate").is_some()
    } else {
        command::find("systemd-inhibit").is_some()
    };
    check("keep-awake backend", caffeine_ok, &mut failures);

    let state = ManagedState::load(paths)?;
    if state.backup_auto || state.backup_hooks {
        check(
            "backup is configured",
            BackupConfig::load(paths).is_ok(),
            &mut failures,
        );
    }
    println!("state directory: {}", paths.state.display());
    println!(
        "caffeine: {}",
        caffeine::configured(paths)?
            .map(|m| m.as_str())
            .unwrap_or("off")
    );
    Ok(if failures == 0 { 0 } else { 1 })
}

fn check(label: &str, passed: bool, failures: &mut i32) {
    if passed {
        println!("[ok] {label}");
    } else {
        println!("[fail] {label}");
        *failures += 1;
    }
}

pub fn status(paths: &Paths) -> Result<()> {
    let doc = codex_config::load(paths)?;
    println!(
        "model = {}",
        doc.get("model").and_then(Item::as_str).unwrap_or("native")
    );
    println!(
        "effort = {}",
        doc.get("model_reasoning_effort")
            .and_then(Item::as_str)
            .unwrap_or("native")
    );
    crate::notify::status(paths)?;
    crate::caffeine::status(paths)?;
    crate::backup::status(paths)?;
    Ok(())
}
