use crate::backup;
use crate::caffeine::{self, Mode};
use crate::command;
use crate::doctor;
use crate::launcher;
use crate::notify;
use crate::paths::Paths;
use crate::profiles;
use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::fs;

pub fn dispatch(paths: &Paths, args: Vec<OsString>) -> Result<i32> {
    let first = args.first().and_then(|value| value.to_str());
    match first {
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(0)
        }
        Some("version") | Some("--version") | Some("-V") => {
            println!("codexify {}", crate::VERSION);
            Ok(0)
        }
        Some("use") => {
            let target = text_arg(&args, 1, "usage: codexify use <model|preset> [effort]")?;
            profiles::use_target(paths, target, args.get(2).and_then(|value| value.to_str()))?;
            Ok(0)
        }
        Some("reset") => {
            profiles::reset(paths)?;
            Ok(0)
        }
        Some("profile") => profile(paths, &args[1..]),
        Some("notify") => notification(paths, &args[1..]),
        Some("caffeine") => caffeine_command(paths, &args[1..]),
        Some("backup") => backup_command(paths, &args[1..]),
        Some("sync") => passthrough("conforme", &args[1..]),
        Some("status") => {
            doctor::status(paths)?;
            Ok(0)
        }
        Some("doctor") => doctor::doctor(paths),
        Some("config") => {
            println!("Codex config: {}", paths.config().display());
            println!("Codex hooks:  {}", paths.hooks().display());
            println!("Codexify:    {}", paths.state.display());
            Ok(0)
        }
        Some("purge") => purge(paths),
        _ => launch(paths, args),
    }
}

fn profile(paths: &Paths, args: &[OsString]) -> Result<i32> {
    match args.first().and_then(|value| value.to_str()) {
        Some("list") => profiles::list(paths)?,
        Some("show") => profiles::show(
            paths,
            text_arg(args, 1, "usage: codexify profile show <name>")?,
        )?,
        Some("add") => profiles::add(
            paths,
            text_arg(
                args,
                1,
                "usage: codexify profile add <name> <model> [effort]",
            )?,
            text_arg(
                args,
                2,
                "usage: codexify profile add <name> <model> [effort]",
            )?,
            args.get(3).and_then(|value| value.to_str()),
        )?,
        Some("use") => profiles::use_profile(
            paths,
            text_arg(args, 1, "usage: codexify profile use <name>")?,
        )?,
        Some("remove") => profiles::remove(
            paths,
            text_arg(args, 1, "usage: codexify profile remove <name>")?,
        )?,
        _ => bail!("usage: codexify profile list|show|add|use|remove"),
    }
    Ok(0)
}

fn notification(paths: &Paths, args: &[OsString]) -> Result<i32> {
    match args.first().and_then(|value| value.to_str()) {
        Some("on") => notify::on(paths, &std::env::current_exe()?)?,
        Some("off") => notify::off(paths)?,
        Some("status") => notify::status(paths)?,
        Some("test") => notify::test()?,
        Some("emit") => {
            let source_index = args.iter().position(|arg| arg == "--source");
            let source = source_index
                .and_then(|index| args.get(index + 1))
                .and_then(|value| value.to_str())
                .unwrap_or("codex");
            let payload = if source == "codex" {
                args.last()
                    .and_then(|value| value.to_str())
                    .filter(|value| *value != "codex")
            } else {
                None
            };
            notify::emit(paths, source, payload)?;
        }
        _ => bail!("usage: codexify notify on|off|status|test"),
    }
    Ok(0)
}

fn caffeine_command(paths: &Paths, args: &[OsString]) -> Result<i32> {
    match args.first().and_then(|value| value.to_str()) {
        Some("on") => {
            let mode = args
                .get(1)
                .and_then(|value| value.to_str())
                .map(Mode::parse)
                .transpose()?
                .unwrap_or(Mode::System);
            caffeine::set(paths, Some(mode))?;
        }
        Some("off") => caffeine::set(paths, None)?,
        Some("status") => caffeine::status(paths)?,
        _ => bail!("usage: codexify caffeine on [system|display]|off|status"),
    }
    Ok(0)
}

fn backup_command(paths: &Paths, args: &[OsString]) -> Result<i32> {
    match args.first().and_then(|value| value.to_str()) {
        Some("init") => backup::init(
            paths,
            text_arg(
                args,
                1,
                "usage: codexify backup init repo|gist <target> [--additive]",
            )?,
            text_arg(
                args,
                2,
                "usage: codexify backup init repo|gist <target> [--additive]",
            )?,
            has(args, "--additive"),
        )?,
        Some("push") => backup::push(
            paths,
            has(args, "--quiet"),
            option_value(args, "--timeout")
                .map(str::parse)
                .transpose()?,
        )?,
        Some("pull") | Some("import") => {
            backup::pull(paths, has(args, "--yes"), has(args, "--additive"))?
        }
        Some("status") => backup::status(paths)?,
        Some("auto") => match args.get(1).and_then(|value| value.to_str()) {
            Some("on") => backup::set_auto(paths, true)?,
            Some("off") => backup::set_auto(paths, false)?,
            Some("status") | None => backup::status(paths)?,
            _ => bail!("usage: codexify backup auto on|off|status"),
        },
        Some("hooks") => match args.get(1).and_then(|value| value.to_str()) {
            Some("on") => backup::set_hooks(paths, &std::env::current_exe()?, true)?,
            Some("off") => backup::set_hooks(paths, &std::env::current_exe()?, false)?,
            Some("status") | None => backup::status(paths)?,
            _ => bail!("usage: codexify backup hooks on|off|status"),
        },
        Some("off") => backup::off(paths)?,
        _ => bail!("usage: codexify backup init|push|pull|import|status|auto|hooks|off"),
    }
    Ok(0)
}

fn launch(paths: &Paths, args: Vec<OsString>) -> Result<i32> {
    let mut forwarded = Vec::with_capacity(args.len());
    let mut mode = None;
    for arg in args {
        if arg == "--caffeine" {
            mode = Some(Mode::System);
        } else if let Some(value) = arg
            .to_str()
            .and_then(|value| value.strip_prefix("--caffeine="))
        {
            mode = Some(Mode::parse(value)?);
        } else {
            forwarded.push(arg);
        }
    }
    launcher::run(paths, &forwarded, mode)
}

fn passthrough(program: &str, args: &[OsString]) -> Result<i32> {
    let executable =
        command::find(program).with_context(|| format!("{program} was not found in PATH"))?;
    Ok(command::exit_code(command::run_status(&executable, args)?))
}

fn purge(paths: &Paths) -> Result<i32> {
    let state = crate::state::ManagedState::load(paths)?;
    if state.notifications_enabled {
        notify::off(paths)?;
    } else {
        notify::remove_hook(paths)?;
    }
    if state.backup_hooks {
        backup::set_hooks(paths, &std::env::current_exe()?, false)?;
    }
    if paths.state.exists() {
        fs::remove_dir_all(&paths.state)?;
        println!(
            "Removed {}. Backups inside that directory are not recoverable.",
            paths.state.display()
        );
    } else {
        println!("Codexify state is already absent.");
    }
    Ok(0)
}

fn text_arg<'a>(args: &'a [OsString], index: usize, usage: &str) -> Result<&'a str> {
    args.get(index)
        .and_then(|value| value.to_str())
        .with_context(|| usage.to_owned())
}

fn has(args: &[OsString], flag: &str) -> bool {
    args.iter().any(|value| value == flag)
}

fn option_value<'a>(args: &'a [OsString], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|value| value == flag)
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.to_str())
}

pub fn print_help() {
    println!(
        "codexify {version}\n\nA practical Codex CLI toolbelt. Unknown options and commands are passed to Codex.\n\nUSAGE:\n  codexify [--caffeine[=system|display]] [CODEX_ARGS...]\n  codexify <COMMAND>\n\nCOMMANDS:\n  use <model|preset> [effort]       Select a model or editable preset\n  reset                             Restore Codex's native model settings\n  profile list|show|add|use|remove Manage editable profiles\n  notify on|off|status|test         Manage completion and approval notifications\n  caffeine on|off|status            Manage persistent keep-awake mode\n  backup init|push|pull|import      Back up the global allow-list\n  backup status|auto|hooks|off      Manage bounded backup automation\n  sync [ARGS...]                    Delegate project synchronization to Conforme\n  status                            Show the effective Codexify state\n  doctor                            Diagnose dependencies and configuration\n  config                            Print configuration paths\n  purge                             Detach integrations and remove Codexify state\n  help                              Show this help\n  version                           Show the version\n\nPRESETS:\n  quality · balanced · fast\n\nNo configuration is changed until an explicit management command is run.",
        version = crate::VERSION
    );
}
