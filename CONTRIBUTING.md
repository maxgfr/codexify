# Contributing

Thanks for improving Codexify. Please open an issue before a broad behavioral or configuration-format change so the reversible migration can be designed first.

## Setup

Install a current stable Rust toolchain, Codex CLI, and Conforme. Then run:

```sh
cargo test --all-features
cargo run -- --help
```

Tests must isolate `CODEX_HOME` and `CODEXIFY_STATE_DIR` in temporary directories. Never use a developer's real Codex config as a fixture.

## Pull requests

- Keep the sidecar and reversibility invariants in `AGENTS.md` intact.
- Include tests for success, failure, idempotence, and restoration paths.
- Preserve unknown config and existing hooks.
- Update README/help/examples when command behavior changes.
- Run formatting, Clippy, tests, and a locked release build before submitting.

Commits should be focused and use clear imperative messages. By contributing, you agree that your contribution is licensed under the MIT License.
