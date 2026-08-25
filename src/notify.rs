use crate::codex_config;
use crate::command;
use crate::fsutil::{atomic_write, read_or_empty};
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use toml_edit::{Array, Item, Value as TomlValue};

const STATUS_MESSAGE: &str = "Codexify Action Required notification";

pub fn on(paths: &Paths, executable: &std::path::Path) -> Result<()> {
    if !executable.is_absolute() {
        bail!("Codexify notification callbacks require an absolute binary path");
    }
    let mut state = ManagedState::load(paths)?;
    let mut doc = codex_config::load(paths)?;
    codex_config::capture(doc.get("notify"), &mut state.notify);
    if !state.tui_notifications.captured {
        state.tui_table_existed = doc.get("tui").is_some();
    }
    let current_tui = doc
        .get("tui")
        .and_then(|item| item.get("notifications"))
        .cloned();
    codex_config::capture(current_tui.as_ref(), &mut state.tui_notifications);
    let hooks = build_hook_document(paths, executable)?;
    state.notifications_enabled = true;
    state.save(paths)?;

    let mut callback = Array::new();
    callback.push(executable.to_string_lossy().as_ref());
    callback.push("notify");
    callback.push("emit");
    callback.push("--source");
    callback.push("codex");
    doc["notify"] = Item::Value(TomlValue::Array(callback));

    let notifications = merged_notifications(current_tui.as_ref());
    doc["tui"]["notifications"] = Item::Value(TomlValue::Array(notifications));
    codex_config::save(paths, &doc)?;
    write_hook_document(paths, &hooks)?;
    println!("Notifications enabled.");
    println!("Open /hooks in Codex and trust the new PermissionRequest hook if prompted.");
    Ok(())
}

fn merged_notifications(item: Option<&Item>) -> Array {
    let mut values = Vec::new();
    if let Some(array) = item.and_then(Item::as_array) {
        for value in array.iter().filter_map(|value| value.as_str()) {
            if !values.iter().any(|current| current == value) {
                values.push(value.to_owned());
            }
        }
    }
    for required in ["agent-turn-complete", "approval-requested"] {
        if !values.iter().any(|current| current == required) {
            values.push(required.to_owned());
        }
    }
    let mut result = Array::new();
    for value in values {
        result.push(value);
    }
    result
}

pub fn off(paths: &Paths) -> Result<()> {
    let mut state = ManagedState::load(paths)?;
    let mut doc = codex_config::load(paths)?;
    codex_config::restore_key(&mut doc, "notify", &mut state.notify)?;
    codex_config::restore_table_key(
        &mut doc,
        "tui",
        "notifications",
        &mut state.tui_notifications,
    )?;
    if !state.tui_table_existed
        && doc
            .get("tui")
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.is_empty())
    {
        doc.remove("tui");
    }
    codex_config::save(paths, &doc)?;
    remove_hook(paths)?;
    state.notifications_enabled = false;
    state.save(paths)?;
    println!("Notifications disabled and previous callbacks restored.");
    Ok(())
}

pub fn status(paths: &Paths) -> Result<()> {
    let state = ManagedState::load(paths)?;
    let doc = codex_config::load(paths)?;
    let callback = doc
        .get("notify")
        .and_then(Item::as_array)
        .and_then(|array| array.get(0))
        .and_then(|value| value.as_str());
    let absolute = callback.is_some_and(|value| std::path::Path::new(value).is_absolute());
    let has_tilde = doc
        .get("notify")
        .is_some_and(|item| item.to_string().contains('~'));
    println!(
        "{}",
        if state.notifications_enabled {
            "on"
        } else {
            "off"
        }
    );
    println!("callback_absolute = {absolute}");
    println!("callback_contains_tilde = {has_tilde}");
    println!("permission_hook = {}", has_owned_hook(paths)?);
    Ok(())
}

pub fn test() -> Result<()> {
    deliver("Codexify", "Notifications are working")?;
    println!("Test notification sent.");
    Ok(())
}

pub fn emit(paths: &Paths, source: &str, payload_arg: Option<&str>) -> Result<()> {
    let payload = if let Some(raw) = payload_arg {
        raw.to_owned()
    } else {
        let mut raw = String::new();
        std::io::stdin().read_to_string(&mut raw)?;
        raw
    };
    let parsed: Value = serde_json::from_str(payload.trim()).unwrap_or_else(|_| json!({}));
    if is_subagent(&parsed) {
        return Ok(());
    }
    let (title, body) = if source == "hook" {
        let tool = parsed
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("Codex");
        let description = parsed
            .pointer("/tool_input/description")
            .and_then(Value::as_str)
            .unwrap_or("Approval requested");
        ("Codex — Action Required", format!("{tool}: {description}"))
    } else {
        let body = parsed
            .get("last-assistant-message")
            .or_else(|| parsed.get("last_assistant_message"))
            .and_then(Value::as_str)
            .unwrap_or("Agent turn complete")
            .chars()
            .take(180)
            .collect();
        ("Codex", body)
    };
    let delivered = deliver(title, &body);
    if source == "codex" {
        chain_previous(paths, payload.trim());
    }
    delivered
}

fn is_subagent(payload: &Value) -> bool {
    payload.get("agent_id").is_some()
        || payload.get("subagent_id").is_some()
        || payload
            .get("is_subagent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.starts_with("Subagent"))
        || payload
            .get("transcript_path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.contains("/subagents/"))
}

fn chain_previous(paths: &Paths, payload: &str) {
    let Ok(state) = ManagedState::load(paths) else {
        return;
    };
    let Some(raw) = state.notify.value else {
        return;
    };
    let Ok(wrapper) = format!("value ={raw}\n").parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let Some(array) = wrapper["value"].as_array() else {
        return;
    };
    let args: Vec<String> = array
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let Some((program, rest)) = args.split_first() else {
        return;
    };
    if program.contains("codexify") {
        return;
    }
    let _ = Command::new(program)
        .args(rest)
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn deliver(title: &str, body: &str) -> Result<()> {
    let backend = detect_backend();
    if let Some(bytes) = terminal_sequence(&backend, title, body) {
        if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
            tty.write_all(&bytes)?;
            tty.flush()?;
            return Ok(());
        }
    }
    native_notification(title, body)
}

pub fn detect_backend() -> String {
    backend_for(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var_os("KITTY_WINDOW_ID").is_some(),
    )
}

pub fn backend_for(term_program: Option<&str>, kitty: bool) -> String {
    if kitty {
        return "kitty".to_owned();
    }
    let term = term_program.unwrap_or_default().to_ascii_lowercase();
    if term.contains("ghostty") {
        "ghostty".to_owned()
    } else if term.contains("iterm") {
        "iterm2".to_owned()
    } else if term.contains("wezterm") {
        "wezterm".to_owned()
    } else if term.contains("apple_terminal") {
        "terminal.app".to_owned()
    } else {
        "native".to_owned()
    }
}

pub fn terminal_sequence(backend: &str, title: &str, body: &str) -> Option<Vec<u8>> {
    let title = sanitize(title);
    let body = sanitize(body);
    match backend {
        "ghostty" | "iterm2" | "wezterm" => {
            Some(format!("\x1b]9;{title}: {body}\x07").into_bytes())
        }
        "kitty" => Some(
            format!("\x1b]99;i=codexify:d=0;{title}\x1b\\\x1b]99;i=codexify:p=body;{body}\x1b\\")
                .into_bytes(),
        ),
        _ => None,
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\x07' | '\x1b' | '\r' | '\n'))
        .collect()
}

fn native_notification(title: &str, body: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        let (program, args) = native_command_spec("macos", false, title, body);
        let status = Command::new(program).args(args).status()?;
        if status.success() {
            return Ok(());
        }
    }
    if is_wsl() && command::find("powershell.exe").is_some() {
        let (program, args) = native_command_spec("linux", true, title, body);
        let status = Command::new(program).args(args).status()?;
        if status.success() {
            return Ok(());
        }
    }
    if let Some(notify_send) = command::find("notify-send") {
        let (_, args) = native_command_spec("linux", false, title, body);
        let status = Command::new(notify_send).args(args).status()?;
        if status.success() {
            return Ok(());
        }
    }
    bail!("no supported notification backend is available")
}

pub fn native_command_spec(
    platform: &str,
    wsl: bool,
    title: &str,
    body: &str,
) -> (String, Vec<String>) {
    if platform == "macos" {
        let script = format!(
            "display notification {} with title {}",
            apple_script_string(body),
            apple_script_string(title)
        );
        return ("osascript".to_owned(), vec!["-e".to_owned(), script]);
    }
    if wsl {
        let escaped_body = body.replace('\'', "''");
        let escaped_title = title.replace('\'', "''");
        let script = format!(
            "[void][System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms'); [System.Windows.Forms.MessageBox]::Show('{escaped_body}','{escaped_title}')"
        );
        return (
            "powershell.exe".to_owned(),
            vec![
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                script,
            ],
        );
    }
    (
        "notify-send".to_owned(),
        vec![title.to_owned(), body.to_owned()],
    )
}

fn apple_script_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/version")
            .is_ok_and(|value| value.to_ascii_lowercase().contains("microsoft"))
}

fn build_hook_document(paths: &Paths, executable: &std::path::Path) -> Result<Value> {
    let mut root: Value = match read_or_empty(&paths.hooks())?.trim() {
        "" => json!({"hooks": {}}),
        raw => serde_json::from_str(raw)
            .with_context(|| format!("failed to parse {}", paths.hooks().display()))?,
    };
    remove_owned_from_json(&mut root);
    let events = root
        .as_object_mut()
        .context("hooks.json root must be an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let permission = events
        .as_object_mut()
        .context("hooks must be an object")?
        .entry("PermissionRequest")
        .or_insert_with(|| json!([]));
    let array = permission
        .as_array_mut()
        .context("hooks.PermissionRequest must be an array")?;
    let executable_string = executable.to_string_lossy();
    let quoted = shell_words::quote(executable_string.as_ref());
    array.push(json!({
        "matcher": "*",
        "hooks": [{
            "type": "command",
            "command": format!("{quoted} notify emit --source hook"),
            "async": true,
            "timeout": 5,
            "statusMessage": STATUS_MESSAGE
        }]
    }));
    Ok(root)
}

fn write_hook_document(paths: &Paths, root: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(root)?;
    bytes.push(b'\n');
    atomic_write(&paths.hooks(), &bytes, &paths.state)
}

pub fn remove_hook(paths: &Paths) -> Result<()> {
    let raw = read_or_empty(&paths.hooks())?;
    if raw.trim().is_empty() {
        return Ok(());
    }
    let mut root: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", paths.hooks().display()))?;
    if !remove_owned_from_json(&mut root) {
        return Ok(());
    }
    let mut bytes = serde_json::to_vec_pretty(&root)?;
    bytes.push(b'\n');
    atomic_write(&paths.hooks(), &bytes, &paths.state)
}

fn remove_owned_from_json(root: &mut Value) -> bool {
    let Some(groups) = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .and_then(|events| events.get_mut("PermissionRequest"))
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = serde_json::to_vec(&*groups).ok();
    for group in groups.iter_mut() {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            handlers.retain(|handler| {
                handler.get("statusMessage").and_then(Value::as_str) != Some(STATUS_MESSAGE)
            });
        }
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|handlers| !handlers.is_empty())
    });
    before != serde_json::to_vec(&*groups).ok()
}

fn has_owned_hook(paths: &Paths) -> Result<bool> {
    let raw = read_or_empty(&paths.hooks())?;
    if raw.trim().is_empty() {
        return Ok(false);
    }
    let root: Value = serde_json::from_str(&raw)?;
    Ok(root
        .pointer("/hooks/PermissionRequest")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            handler.get("statusMessage").and_then(Value::as_str)
                                == Some(STATUS_MESSAGE)
                        })
                    })
            })
        }))
}

#[cfg(test)]
mod tests {
    use super::{backend_for, native_command_spec, terminal_sequence};

    #[test]
    fn osc9_sequences_are_exact() {
        let expected = b"\x1b]9;Codex: Done\x07".to_vec();
        for backend in ["ghostty", "iterm2", "wezterm"] {
            assert_eq!(
                terminal_sequence(backend, "Codex", "Done"),
                Some(expected.clone())
            );
        }
    }

    #[test]
    fn kitty_sequence_is_exact() {
        assert_eq!(
            terminal_sequence("kitty", "Codex", "Done"),
            Some(
                b"\x1b]99;i=codexify:d=0;Codex\x1b\\\x1b]99;i=codexify:p=body;Done\x1b\\".to_vec()
            )
        );
    }

    #[test]
    fn backend_detection_is_deterministic() {
        assert_eq!(backend_for(Some("ghostty"), false), "ghostty");
        assert_eq!(backend_for(Some("iTerm.app"), false), "iterm2");
        assert_eq!(backend_for(Some("WezTerm"), false), "wezterm");
        assert_eq!(backend_for(Some("Apple_Terminal"), false), "terminal.app");
        assert_eq!(backend_for(Some("anything"), true), "kitty");
    }

    #[test]
    fn os_backend_commands_are_exact() {
        assert_eq!(
            native_command_spec("macos", false, "Codex", "It's done"),
            (
                "osascript".to_owned(),
                vec![
                    "-e".to_owned(),
                    "display notification \"It's done\" with title \"Codex\"".to_owned()
                ]
            )
        );
        assert_eq!(
            native_command_spec("linux", false, "Codex", "Done"),
            (
                "notify-send".to_owned(),
                vec!["Codex".to_owned(), "Done".to_owned()]
            )
        );
        let wsl = native_command_spec("linux", true, "Codex", "It's done");
        assert_eq!(wsl.0, "powershell.exe");
        assert_eq!(&wsl.1[..3], ["-NoProfile", "-NonInteractive", "-Command"]);
        assert!(wsl.1[3].contains("It''s done"));
    }
}
