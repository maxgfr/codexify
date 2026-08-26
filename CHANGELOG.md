## [0.2.4](https://github.com/maxgfr/codexify/compare/v0.2.3...v0.2.4) (2026-08-26)


### Bug Fixes

* keep native Ghostty notifications ([a178e7e](https://github.com/maxgfr/codexify/commit/a178e7ef24cc37cd458ba3b905037d772aa75b05))

## [0.2.3](https://github.com/maxgfr/codexify/compare/v0.2.2...v0.2.3) (2026-08-26)


### Bug Fixes

* diagnose broken Codex hook commands ([2d1ea94](https://github.com/maxgfr/codexify/commit/2d1ea9435aae184c23dff098b85b6908dcabe6ce))

## Unreleased

### Fixed

- Detect malformed hook configuration, missing executables, and relative hook
  script paths in `codexify doctor`.
- Preserve a native notification fallback when Ghostty accepts OSC 777 without
  displaying a visible banner.

## [0.2.2](https://github.com/maxgfr/codexify/compare/v0.2.1...v0.2.2) (2026-08-26)


### Bug Fixes

* preserve terminal application badges ([dc8a8e0](https://github.com/maxgfr/codexify/commit/dc8a8e0c7a02f0e5d8d3adfe9af3babce3991f8c))
## [0.2.1](https://github.com/maxgfr/codexify/compare/v0.2.0...v0.2.1) (2026-08-25)


### Bug Fixes

* mark Ghostty tabs for notifications ([367a17b](https://github.com/maxgfr/codexify/commit/367a17b04d234ff9c5d54aae98533433578371a8))

# [0.2.0](https://github.com/maxgfr/codexify/compare/v0.1.0...v0.2.0) (2026-08-25)


### Features

* automate releases with semantic-release ([f6b29af](https://github.com/maxgfr/codexify/commit/f6b29af2e65f5f473d497c4612b1bca27e85213b))

# Changelog

All notable changes follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and semantic versioning.

## [0.1.0] - 2026-08-25

### Added

- Transparent Codex launcher with exact argument and exit-code forwarding.
- Editable quality, balanced, and fast model profiles with native-config reset.
- Reliable completion and approval notifications across terminal and OS backends.
- Reversible structured edits for Codex TOML and JSON hooks.
- Persistent and one-shot keep-awake modes.
- Allow-listed private-repository and secret-gist config backup with secret scanning.
- Bounded backup automation and hook integration.
- Exact Conforme delegation, status, diagnostics, config paths, and safe purge.
- Linux/macOS CI, four-platform release artifacts, checksums, and Homebrew distribution.

[0.1.0]: https://github.com/maxgfr/codexify/releases/tag/v0.1.0
