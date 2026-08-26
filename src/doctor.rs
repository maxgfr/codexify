use crate::backup::BackupConfig;
use crate::caffeine;
use crate::codex_config;
use crate::command;
use crate::fsutil::read_or_empty;
use crate::paths::Paths;
use crate::state::ManagedState;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;
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

    let hooks = load_hooks(paths);
    check("Codex hooks parse", hooks.is_ok(), &mut failures);
    if let Ok(root) = &hooks {
        check(
            "hook command paths are portable",
            root.as_ref().is_none_or(hook_commands_are_portable),
            &mut failures,
        );
    } else {
        println!("[skip] hook command paths are portable (hooks did not parse)");
    }

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

fn load_hooks(paths: &Paths) -> Result<Option<Value>> {
    let raw = read_or_empty(&paths.hooks())?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&raw)?))
}

fn hook_commands_are_portable(root: &Value) -> bool {
    let Some(events) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    events.values().all(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().all(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().all(|handler| {
                            match handler.get("type").and_then(Value::as_str) {
                                Some("command") => handler
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .is_some_and(hook_command_is_portable),
                                Some("mcp_tool" | "prompt" | "agent") => true,
                                _ => false,
                            }
                        })
                    })
            })
        })
    })
}

fn hook_command_is_portable(command_line: &str) -> bool {
    if has_unquoted_shell_control(command_line) {
        return false;
    }
    let Ok(arguments) = shell_words::split(command_line) else {
        return false;
    };
    command_arguments_are_portable(&arguments)
}

fn has_unquoted_shell_control(command_line: &str) -> bool {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    for character in command_line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && !single_quoted {
            escaped = true;
            continue;
        }
        if character == '\'' && !double_quoted {
            single_quoted = !single_quoted;
            continue;
        }
        if character == '"' && !single_quoted {
            double_quoted = !double_quoted;
            continue;
        }
        if !single_quoted && !double_quoted && matches!(character, ';' | '|' | '&' | '\n' | '\r') {
            return true;
        }
    }
    false
}

fn command_arguments_are_portable(arguments: &[String]) -> bool {
    let arguments = &arguments[arguments
        .iter()
        .take_while(|argument| is_environment_assignment(argument))
        .count()..];
    let Some(executable) = arguments.first() else {
        return false;
    };
    if !command_target_is_usable(executable) {
        return false;
    }

    let interpreter = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(executable);
    match interpreter {
        "env" => env_invocation_is_portable(&arguments[1..]),
        "bash" | "sh" | "zsh" => shell_invocation_is_portable(&arguments[1..]),
        "node" | "nodejs" => interpreter_invocation_is_portable(
            &arguments[1..],
            &["-e", "--eval", "-p", "--print"],
            &["-r", "--require", "--loader", "--import"],
        ),
        name if name == "python"
            || name == "python3"
            || name.strip_prefix("python").is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '.')
            }) =>
        {
            interpreter_invocation_is_portable(&arguments[1..], &["-c", "-m"], &["-W", "-X"])
        }
        "ruby" | "perl" => interpreter_invocation_is_portable(&arguments[1..], &["-e"], &[]),
        "pwsh" | "powershell" => interpreter_invocation_is_portable(
            &arguments[1..],
            &["-Command", "-EncodedCommand"],
            &[],
        ),
        _ => true,
    }
}

fn is_environment_assignment(argument: &str) -> bool {
    let Some((name, _)) = argument.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn env_invocation_is_portable(arguments: &[String]) -> bool {
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        if argument == "-S" || argument == "--split-string" {
            let Some(command_line) = arguments.get(index + 1) else {
                return false;
            };
            return hook_command_is_portable(command_line);
        }
        if argument == "-u" || argument == "--unset" {
            if arguments.get(index + 1).is_none() {
                return false;
            }
            index += 2;
            continue;
        }
        if argument == "-C" || argument == "--chdir" {
            return false;
        }
        if argument.starts_with('-') || argument.contains('=') {
            index += 1;
            continue;
        }
        return command_arguments_are_portable(&arguments[index..]);
    }
    false
}

fn shell_invocation_is_portable(arguments: &[String]) -> bool {
    for (index, argument) in arguments.iter().enumerate() {
        if argument.starts_with('-')
            && !argument.starts_with("--")
            && argument.trim_start_matches('-').contains('c')
        {
            let Some(command_line) = arguments.get(index + 1) else {
                return false;
            };
            if command_line
                .chars()
                .any(|character| matches!(character, ';' | '|' | '&' | '\n' | '\r'))
            {
                return false;
            }
            let Ok(command) = shell_words::split(command_line) else {
                return false;
            };
            if command.first().is_some_and(|name| {
                matches!(
                    name.as_str(),
                    ":" | "break"
                        | "continue"
                        | "echo"
                        | "exit"
                        | "export"
                        | "printf"
                        | "pwd"
                        | "read"
                        | "return"
                        | "set"
                        | "shift"
                        | "test"
                        | "times"
                        | "trap"
                        | "umask"
                        | "unset"
                        | "wait"
                )
            }) {
                return true;
            }
            return command_arguments_are_portable(&command);
        }
    }
    interpreter_invocation_is_portable(arguments, &[], &[])
}

fn interpreter_invocation_is_portable(
    arguments: &[String],
    inline_flags: &[&str],
    value_flags: &[&str],
) -> bool {
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        if argument == "--" {
            return arguments
                .get(index + 1)
                .is_none_or(|script| script_path_is_portable(script));
        }
        if inline_flags.contains(&argument.as_str()) {
            return arguments.get(index + 1).is_some();
        }
        if value_flags.contains(&argument.as_str()) {
            if arguments.get(index + 1).is_none() {
                return false;
            }
            index += 2;
            continue;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        return script_path_is_portable(argument);
    }
    true
}

fn script_path_is_portable(script: &str) -> bool {
    let path = Path::new(script);
    path.is_absolute() && !script.contains('~') && path.is_file()
}

fn command_target_is_usable(target: &str) -> bool {
    let path = Path::new(target);
    if path.is_absolute() {
        !target.contains('~') && is_executable(path)
    } else if target.contains('/') || target.contains('~') {
        false
    } else {
        command::find(target).is_some_and(|path| is_executable(&path))
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

fn check(label: &str, passed: bool, failures: &mut i32) {
    if passed {
        println!("[ok] {label}");
    } else {
        println!("[fail] {label}");
        *failures += 1;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::hook_command_is_portable;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn env_wrappers_do_not_hide_relative_scripts() {
        assert!(!hook_command_is_portable(
            "/usr/bin/env /bin/sh relative-hook"
        ));
    }

    #[test]
    fn top_level_compound_commands_are_rejected() {
        assert!(!hook_command_is_portable(
            "/bin/true && definitely-missing-codexify-hook"
        ));
    }

    #[test]
    fn leading_environment_assignments_are_supported() {
        assert!(hook_command_is_portable("HOOK_MODE=check /usr/bin/true"));
    }

    #[test]
    fn quoted_shell_metacharacters_are_not_treated_as_composition() {
        assert!(hook_command_is_portable("/bin/echo 'value;still-value'"));
    }

    #[test]
    fn shell_inline_commands_allow_options_before_c() {
        assert!(hook_command_is_portable("/bin/sh -eu -c 'exit 0'"));
    }

    #[test]
    fn shell_builtins_that_execute_paths_are_not_blanket_accepted() {
        assert!(!hook_command_is_portable(
            "/bin/sh -c 'exec ./definitely-missing-hook'"
        ));
    }

    #[test]
    fn env_unset_options_consume_their_value() {
        assert!(hook_command_is_portable(
            "/usr/bin/env -u HOOK_MODE /bin/sh -c 'exit 0'"
        ));
    }

    #[test]
    fn compound_shell_commands_are_rejected_when_they_cannot_be_proven_portable() {
        assert!(!hook_command_is_portable(
            "/bin/sh -c 'echo start; ./definitely-missing-hook'"
        ));
    }

    #[test]
    fn long_shell_options_are_not_mistaken_for_c() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), "exit 0\n").unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(hook_command_is_portable(&format!(
            "/bin/bash --norc {}",
            temp.path().display()
        )));
    }

    #[test]
    fn interpreter_option_values_do_not_hide_the_main_script() {
        let temp = tempfile::tempdir().unwrap();
        let node = temp.path().join("node");
        let preload = temp.path().join("preload.mjs");
        fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&preload, "").unwrap();

        assert!(!hook_command_is_portable(&format!(
            "{} --require {} relative-main.mjs",
            node.display(),
            preload.display()
        )));
    }

    #[test]
    fn non_executable_commands_are_rejected() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!hook_command_is_portable(
            temp.path().to_string_lossy().as_ref()
        ));
    }

    #[test]
    fn versioned_python_names_still_validate_the_script_path() {
        let temp = tempfile::tempdir().unwrap();
        let python = temp.path().join("python3.12");
        fs::write(&python, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!hook_command_is_portable(&format!(
            "{} missing-relative.py",
            python.display()
        )));
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
