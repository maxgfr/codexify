# Codexify contributor instructions

## Architecture

- `src/cli.rs` owns dispatch and the intentional boundary between Codexify commands and exact Codex pass-through.
- `src/codex_config.rs`, `src/fsutil.rs`, and `src/state.rs` own reversible, atomic user-config edits.
- `src/profiles.rs` owns model/profile selection.
- `src/notify.rs` owns terminal/OS notification delivery and the managed `PermissionRequest` hook.
- `src/caffeine.rs` and `src/launcher.rs` own child lifetime and keep-awake behavior.
- `src/backup.rs` owns the global allow-list, portability transform, secret gate, Git/gist transports, restore diff, and automation hook.
- `src/doctor.rs` reports state without changing it.
- Project synchronization is always delegated to the installed `conforme` binary.

## Invariants

1. `codex` must remain usable without Codexify. No shell aliases, PATH edits, or dotfile injection.
2. Running Codexify with Codex arguments must preserve argument boundaries and the child's exit code.
3. The native Codex model remains untouched until `use` or `profile use` is explicit.
4. Every edit of `config.toml` or `hooks.json` is structured, backed up, same-directory atomic, and reversible.
5. Never erase unknown TOML keys, comments, notification events, or user hooks.
6. Managed callback paths are absolute and contain no `~`.
7. `PermissionRequest` notification hooks are asynchronous and never return an approval decision.
8. Codexify-owned hook handlers are identified by their command and `statusMessage`; removal must target only those handlers.
9. Backup inclusion is allow-list based. Do not replace it with a deny-list.
10. Never print secret values. A blocked backup reports only relative file, line, and finding type.
11. Backup automation is best-effort and bounded; it may never block Codex startup.
12. `purge` detaches integrations before deleting `~/.codexify`.
13. No required runtime script may live only on a developer machine. Callbacks are internal binary subcommands.

## Changing behavior

- Add a regression test with an `FR-*` tag comment when changing an invariant or user-facing flow.
- Use `CODEX_HOME` and `CODEXIFY_STATE_DIR` in integration tests; never target the developer's real home.
- Notification protocol changes require byte-for-byte tests for every affected terminal.
- Backup changes require coverage of allow-list inclusion, excluded sensitive files, portability, secret output, mirror/additive restore, and local-hook stripping.
- Keep README command syntax, `codexify help`, examples, and changelog synchronized.

## Required gates

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --locked
```

## Release process

Automated via semantic-release on every push to `main`:

1. Use Conventional Commits. `feat:` releases a minor version, `fix:` and `perf:` release a patch, and a `BREAKING CHANGE:` footer releases a major version. Non-release commits such as `docs:`, `test:`, and `ci:` still run the workflow but do not publish.
2. The semantic-release workflow runs every required Rust gate before analyzing commits.
3. `.version-hook.sh` updates `Cargo.toml` and `Cargo.lock`; semantic-release updates `CHANGELOG.md`, creates the release commit, `v<crate-version>` tag, and a draft GitHub release.
4. A successful semantic release calls `release.yml` synchronously. It verifies the tag/crate match, builds four binaries, writes `.sha256` files plus `checksums.txt`, attaches them, and only then publishes the release.
5. `maxgfr/homebrew-tap` checks daily for the latest release and updates the formula. After release changes, run its updater manually when immediate distribution is required.

Never edit the crate version or create a release tag manually for a normal release. `release.yml` retains a manual dispatch for rebuilding assets of an existing tag. If semantic-release fails after preparing the version or tag, run `recover-release.yml` manually with the expected `v<version>`; it reconciles the tag and draft release before rebuilding the assets.
