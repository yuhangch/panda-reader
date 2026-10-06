#!/usr/bin/env python3
"""Generate the community plugin catalog from plugin manifests and payloads."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


REPOSITORY = "yuhangch/panda-reader"
BRANCH = "main"
VERSION_PATTERN = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")


def normalized_bytes(path: Path) -> bytes:
    # Git stores repository text with LF even when a Windows checkout uses CRLF.
    return path.read_bytes().replace(b"\r\n", b"\n")


def generate_index(root: Path) -> dict[str, Any]:
    community_dir = root / "plugins" / "community"
    plugins: list[dict[str, Any]] = []
    seen_ids: set[str] = set()

    for directory in sorted(community_dir.iterdir()):
        manifest_path = directory / "manifest.toml"
        if not directory.is_dir() or not manifest_path.is_file():
            continue

        manifest = tomllib.loads(normalized_bytes(manifest_path).decode("utf-8"))
        plugin_id = manifest.get("id")
        name = manifest.get("name")
        version = manifest.get("version")
        api_version = manifest.get("api_version")
        min_app_version = manifest.get("min_app_version")
        kind = manifest.get("kind")

        if not isinstance(plugin_id, str) or not plugin_id.startswith("community."):
            raise ValueError(f"{manifest_path}: plugin ID must start with 'community.'")
        if plugin_id != directory.name:
            raise ValueError(f"{manifest_path}: directory name must match plugin ID {plugin_id!r}")
        if plugin_id in seen_ids:
            raise ValueError(f"duplicate community plugin ID: {plugin_id}")
        seen_ids.add(plugin_id)

        if not isinstance(name, str) or not name.strip():
            raise ValueError(f"{manifest_path}: name is required")
        if not isinstance(version, str) or not VERSION_PATTERN.fullmatch(version):
            raise ValueError(f"{manifest_path}: version must use semantic version syntax")
        if not isinstance(api_version, int) or api_version < 1:
            raise ValueError(f"{manifest_path}: api_version must be a positive integer")
        if not isinstance(min_app_version, str) or not VERSION_PATTERN.fullmatch(min_app_version):
            raise ValueError(f"{manifest_path}: min_app_version must use semantic version syntax")
        if kind not in {"rules", "wasm"}:
            raise ValueError(f"{manifest_path}: kind must be 'rules' or 'wasm'")

        payload_name = "rules.toml" if kind == "rules" else "plugin.wasm"
        file_entries: dict[str, dict[str, str]] = {}
        for filename in ("manifest.toml", payload_name):
            path = directory / filename
            if not path.is_file():
                raise ValueError(f"{directory}: missing required file {filename}")
            relative_path = path.relative_to(root).as_posix()
            url = (
                f"https://raw.githubusercontent.com/{REPOSITORY}/refs/heads/{BRANCH}/"
                f"{relative_path}"
            )
            file_entries[filename] = {
                "url": url,
                "sha256": hashlib.sha256(normalized_bytes(path)).hexdigest(),
            }

        plugins.append(
            {
                "id": plugin_id,
                "name": name,
                "version": version,
                "api_version": api_version,
                "min_app_version": min_app_version,
                "kind": kind,
                "files": file_entries,
            }
        )

    return {"schema_version": 1, "plugins": plugins}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail if index.json does not match the current plugin manifests and payloads",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    index_path = root / "plugins" / "community" / "index.json"
    try:
        generated = json.dumps(generate_index(root), ensure_ascii=False, indent=2) + "\n"
        if args.check:
            if not index_path.is_file() or index_path.read_text(encoding="utf-8") != generated:
                print(
                    "Community plugin index is out of date. "
                    "Run: python scripts/generate_community_plugin_index.py",
                    file=sys.stderr,
                )
                return 1
            print("Community plugin index is up to date.")
            return 0

        index_path.write_text(generated, encoding="utf-8", newline="\n")
        print(f"Generated {index_path.relative_to(root)}")
        return 0
    except (OSError, UnicodeError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"Cannot generate community plugin index: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
