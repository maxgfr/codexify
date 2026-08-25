#!/bin/sh
# Called by semantic-release to keep the crate and lockfile versions aligned.
set -eu

VERSION="${1:?missing release version}"
case "$VERSION" in
  *[!0-9.]* | .* | *.)
    echo "invalid semantic version: $VERSION" >&2
    exit 2
    ;;
esac

TEMP_CARGO="Cargo.toml.semantic-release.$$"
trap 'rm -f "$TEMP_CARGO"' EXIT HUP INT TERM

awk -v version="$VERSION" '
  !updated && /^version = "[^"]+"$/ {
    print "version = \"" version "\""
    updated = 1
    next
  }
  { print }
  END { if (!updated) exit 2 }
' Cargo.toml > "$TEMP_CARGO"
mv "$TEMP_CARGO" Cargo.toml

# Cargo updates only the workspace package record while retaining locked deps.
cargo check --quiet

awk -v version="$VERSION" '
  /^name = "codexify"$/ { codexify = 1; next }
  codexify && /^version = / {
    if ($0 == "version = \"" version "\"") found = 1
    codexify = 0
  }
  END { exit(found ? 0 : 1) }
' Cargo.lock
