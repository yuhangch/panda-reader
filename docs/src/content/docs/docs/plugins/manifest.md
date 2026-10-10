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
stage = "cleanup" # "prepare" or "resolve_url" for Wasm plugins
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

Wasm URL resolvers run before the host downloads an article page. They must declare
`article_url_write`; plugins that perform requests must also declare `network_request` and an exact
`network_hosts` allowlist, for example `network_hosts = ["news.google.com"]`. Network requests are
available only during `resolve_url`, use HTTPS, and are made by the host with size and time limits.
Wildcard network hosts and IP literals are rejected.

See the [rules example](https://github.com/yuhangch/panda-reader/tree/main/docs/plugins/examples/rules)
for a complete manifest and payload.
