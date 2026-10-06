# Article plugins

Panda Reader plugins transform article HTML on the host. The host owns HTML parsing, URL
handling, extraction, and final sanitization. Plugins have no network, filesystem, database,
process, or UI access. A failing plugin is rolled back and the rest of the content pipeline
continues.

## Plugin directory

Each plugin is one directory with `manifest.toml` and either `rules.toml` or `plugin.wasm`.
In Settings → Plugins, browse to a directory or ZIP archive, paste a local path, or provide an
HTTPS URL to a ZIP archive. The application copies and validates the plugin before making it
available. User plugins cannot replace the reserved `builtin.*` IDs.

The Community plugins section reads [`plugins/community/index.json`](../../plugins/community/index.json)
over HTTPS. It offers install and versioned update actions. The app verifies each downloaded file's
SHA-256 and checks the manifest against the catalog before replacing an installed plugin. Plugin
updates are checked with the app's update check and can also be checked from Settings → Plugins;
installation remains an explicit action.
Debug builds read the catalog and plugin files from this workspace, so community changes can be
tested before they are published. Release builds use the HTTPS catalog and file URLs.

ZIP imports accept one plugin per archive. Archives are limited to 8 MiB compressed and
uncompressed, with at most 64 entries. Only the manifest and plugin payload files are staged;
archive paths are validated and never extracted directly into the application data directory.
Remote imports require HTTPS and are limited to 8 MiB with a 30-second request timeout.

Community publisher rules live in [`plugins/community`](../../plugins/community), including
ScienceNet, IT Home, China News Network, WeChat, and QbitAI. This directory contains the authoring
guide and examples. Contributors can maintain plugins here or publish them independently for
users to import.
For agent-assisted plugin authoring, see the repository skill at
[`panda-reader-plugin`](../../.agents/skills/panda-reader-plugin/SKILL.md).
The community catalog is generated from plugin manifests and payloads with
`python scripts/generate_community_plugin_index.py`; CI checks that it stays current.

```toml
id = "example.site-cleanup"
name = "Example site cleanup"
version = "1.0.0"
api_version = 1
min_app_version = "0.2.0"
kind = "rules" # or "wasm"
domains = ["example.com", "*.example.net"]
path_prefixes = ["/articles/"]
stage = "cleanup" # or "prepare"
priority = 20
capabilities = ["document_read", "document_write"]
```

An exact domain matches only that hostname. `*.example.net` matches both the root domain and
its subdomains. A path prefix is matched against the parsed URL path. Plugins run from higher
priority to lower priority, then by plugin ID. Prepare plugins may repair source HTML or set
the extracted article body. Cleanup plugins run after RSS content or extracted web content is
available.

## Versioning and updates

Keep `id` stable for the lifetime of a plugin and use `version` as its semantic version
(`major.minor.patch`). `api_version` is separate: it identifies the host plugin API and is
checked when a plugin is imported. The current app displays `version` and importing another
directory with the same `id` replaces the installed copy, but it does not parse plugin semantic
versions or update plugins automatically. Users install updates by importing the newer copy.

## Declarative rules

Rules are evaluated in file order. Selectors are validated while the plugin is loaded. Supported
actions are selecting a body, removing matching nodes, removing nodes with exact normalized text,
setting/removing an attribute, and fixing lazy-loaded image attributes. See
[`examples/rules`](examples/rules).

## Wasm and Rust SDK

Wasm plugins target `wasm32-unknown-unknown`, export `panda_process(stage: i32) -> i32`, and use
the `panda_v1` JSON host ABI. The Rust SDK wraps the ABI and exports the entry point for you.
See [`examples/json-recovery`](examples/json-recovery) for a prepare plugin that recovers article
HTML stored in a page's JSON script element.

The host currently limits a module to 4 MiB, decoded HTML to 8 MiB, the document to 100,000
nodes, guest memory to 16 MiB, each stage to 10 million Wasm fuel, and a plugin invocation to
10,000 host calls. JSON requests and responses are limited to 2 MiB each. Fuel bounds executed
Wasm instructions; it is not a wall-clock timeout. Unknown imports, invalid pointers, missing
capabilities, and invalid node handles fail that plugin invocation.

The host operations cover context, selector queries, node text/HTML/attributes, node removal,
attribute changes, HTML replacement, article body selection, URL resolution, and logging. DOM
nodes stay in the host; node handles are valid only for the current plugin call.

## Local fixture runner

Run both stages against a local HTML fixture. It does not fetch URLs:

```sh
cargo run -p panda-plugins --bin panda-plugin-runner -- \
  ./plugins https://example.com/articles/42 "Example title" \
  docs/plugins/examples/rules/fixture.html
```

To run the included selector rules, pass `docs/plugins/examples/rules` as the plugin directory.

Processed HTML is written to standard output. Diagnostics go to standard error.
