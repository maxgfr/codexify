# codexify

`codexify` is a practical sidecar for the [Codex CLI](https://learn.chatgpt.com/docs/codex-cli): model profiles, reliable **Action Required** notifications, global config backup, keep-awake sessions, diagnostics, and project sync through [Conforme](https://github.com/maxgfr/conforme).

Codex remains fully usable on its own. Running `codexify` without a management command simply launches `codex` and forwards every argument and the exit code. Codexify never edits shell startup files or project dotfiles.

## Install

With Homebrew (macOS and Linux):

```sh
brew install maxgfr/tap/codexify
codexify doctor
```

Homebrew installs Conforme as a dependency. To build from source:

```sh
cargo install --locked --git https://github.com/maxgfr/codexify
```

Prebuilt release binaries are available for macOS and Linux on ARM64 and x86-64.

## Launch Codex

```sh
codexify
codexify exec "explain this repository"
codexify --model gpt-5.6-luna
codexify --caffeine
codexify --caffeine=display exec "run the test suite"
```

Unknown options and commands are forwarded unchanged. `--caffeine` is the only launch option consumed by Codexify.

## Models and profiles

Codex's native model selection is untouched until you opt in:

```sh
codexify use quality
codexify use gpt-5.6-terra high
codexify reset

codexify profile list
codexify profile show balanced
codexify profile add review gpt-5.6 high
codexify profile use review
codexify profile remove review
```

The editable presets live in `~/.codexify/profiles.toml`:

| Preset | Default model | Effort |
| --- | --- | --- |
| `quality` | `gpt-5.6` | `xhigh` |
| `balanced` | `gpt-5.6` | `medium` |
| `fast` | `gpt-5.6-luna` | `medium` |

Model names and availability evolve; edit the file to match your account. The [official model guide](https://developers.openai.com/api/docs/guides/latest-model) explains the current GPT-5.6 aliases. `reset` restores the exact pre-Codexify `model` and `model_reasoning_effort` values, including absence.

## Reliable notifications

```sh
codexify notify on
codexify notify status
codexify notify test
codexify notify off
```

`notify on` repairs both common causes of missing Action Required alerts:

- it merges `agent-turn-complete` and `approval-requested` into `[tui].notifications` without removing custom events;
- it replaces fragile commands such as `notify = ["node", "~/.codex/notify.mjs"]` with the absolute installed path to `codexify notify emit`.

It also appends an asynchronous `PermissionRequest` hook to `~/.codex/hooks.json`. The hook only displays a notification: it emits no allow/deny decision, so the normal approval prompt continues. Existing hooks and the previous `notify` callback (including Codex Desktop callbacks) are preserved; Codexify chains the callback while enabled and restores it on `notify off`.

Codex requires changed non-managed hooks to be reviewed. After enabling notifications, open `/hooks` in Codex and trust the Codexify entry if prompted. This follows the [official hook trust flow](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks). The underlying keys are documented in the [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

### Terminal support

- Ghostty, iTerm2, and WezTerm: OSC 9 notifications.
- Kitty: OSC 99 title/body notifications.
- Terminal.app and other macOS terminals: Notification Center through `osascript`.
- Linux: terminal protocol when detected, otherwise `notify-send`.
- WSL: PowerShell notification fallback.

Codexify discards payloads identified as sub-agent events to avoid notification storms. For Ghostty, ensure notifications are allowed for Ghostty in **System Settings → Notifications**, then run `codexify notify test` from the terminal.

## Keep awake

```sh
codexify caffeine on           # prevent system sleep for future launches
codexify caffeine on display   # keep the display awake
codexify caffeine status
codexify caffeine off
```

On macOS, Codexify wraps the child in `caffeinate`; on Linux it uses `systemd-inhibit`. The lock exists only while the child process is alive and is released on success, error, or signal-driven termination. If no backend exists, Codex still launches and Codexify prints a warning.

## Global backup

Backups use a strict allow-list: `AGENTS.md`, `config.toml`, `*.config.toml`, `hooks.json`, and files under `hooks/`, `rules/`, and `skills/`.

They never include `auth.json`, sessions, history, logs, memories, goals, SQLite databases, caches, Browser/Computer Use data, plugin downloads, or any other unlisted file.

### Private repository

Create a private empty repository, then configure its Git URL:

```sh
gh repo create my-codex-config --private
codexify backup init repo git@github.com:YOUR_USER/my-codex-config.git
codexify backup push
codexify backup status
```

### Secret gist

Create a secret gist directly (or pass an existing gist ID):

```sh
codexify backup init gist new
# or: codexify backup init gist GIST_ID
codexify backup push
```

Gist files are flattened with a manifest so directories round-trip. Repository and gist data replace the local allow-list by default. Use additive mode to retain local allow-listed files absent remotely:

```sh
codexify backup pull             # show diff, ask, mirror restore
codexify backup pull --additive  # show diff, ask, additive restore
codexify backup import --yes     # non-interactive restore
```

Before push, absolute home paths become the escaped portable `$CODEXIFY_HOME$` token and local Codexify callbacks are removed. Literal token text and binary assets round-trip safely; paths are expanded and callbacks reattached after restore. A secret scan blocks push and prints only file, line, and finding type—never the matched value.

Automation is explicit and bounded:

```sh
codexify backup auto on     # best-effort 2-second launch-time push
codexify backup hooks on    # async Stop hook, 5-second timeout
codexify backup hooks off
codexify backup auto off
codexify backup off
```

Backup failures never prevent Codex from launching.

## Project sync with Conforme

Codexify deliberately does not own project configuration. It forwards arguments and exit status to Conforme:

```sh
codexify sync
codexify sync --dry-run
codexify sync --from claude --only cursor,codex
```

See [Conforme's documentation](https://github.com/maxgfr/conforme) for supported agents and project formats.

## Diagnostics and state

```sh
codexify status
codexify doctor
codexify config
codexify --version
```

Codexify owns only `~/.codexify`. Every write to `~/.codex/config.toml` or `~/.codex/hooks.json` is parsed structurally, backed up under `~/.codexify/backups`, written through a same-directory temporary file, synced, and atomically renamed. Comments, ordering, unknown TOML keys, and non-Codexify hooks are preserved.

## Uninstall and restore

Detach managed configuration before uninstalling:

```sh
codexify notify off
codexify backup off
codexify reset
codexify purge
brew uninstall codexify
```

`purge` detaches notification and backup hooks before deleting `~/.codexify`. Historical `.bak` files inside that directory are removed and cannot be recovered afterward. `codex`, its authentication, sessions, history, and all unrelated configuration remain untouched.

If a write was interrupted or manual recovery is needed, inspect `~/.codexify/backups/` and copy the desired timestamped file back to `~/.codex/`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md). Codexify is MIT licensed.
