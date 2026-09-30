#!/usr/bin/env bash
# Print the workspace package version from the root Cargo.toml.
# Match `version = "…"`, not `version.workspace = true`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(grep -m1 '^version = "' "$ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "could not read workspace version from Cargo.toml (got '$VERSION')" >&2
  exit 1
fi
printf '%s\n' "$VERSION"
