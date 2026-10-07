---
title: Plugin manifest
description: Define plugin identity, matching rules, execution stage, and capabilities.
---

Every plugin directory contains a `manifest.toml` and one payload. The manifest describes what
the plugin is, which pages it can handle, and when it should run.

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

Keep `id` stable and increment `version` when plugin behavior changes. `api_version` identifies
the host plugin API and is checked when the plugin is imported. `min_app_version` can prevent
installation on older app versions. `kind` must match the payload format.

An exact domain matches only that hostname. A wildcard such as `*.example.net` matches the root
domain and its subdomains. `path_prefixes` are matched against the parsed URL path. If specified,
both the domain and path must match.

`prepare` plugins can repair source HTML before extraction; `cleanup` plugins process RSS content
or extracted article content. Plugins run by descending `priority`, then by plugin ID. Capabilities
declare which host document operations the plugin needs; undeclared operations are rejected.

See the [rules example](https://github.com/yuhangch/panda-reader/tree/main/docs/plugins/examples/rules)
for a complete manifest and payload.
