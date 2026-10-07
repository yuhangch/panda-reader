---
title: WASM plugins
description: Build a site-specific article transformation using the constrained host API.
---

Use a WASM plugin when the transformation is too involved for declarative rules. Plugins target
`wasm32-unknown-unknown`, export `panda_process(stage: i32) -> i32`, and communicate with the app
through the `panda_v1` JSON host ABI. The Rust SDK wraps the ABI and exports the entry point.

The host provides operations for reading context, querying and editing document nodes, replacing
HTML, selecting article content, resolving URLs, and logging. DOM nodes stay in the host; handles
are valid only for one plugin invocation.

See the repository's
[JSON recovery example](https://github.com/yuhangch/panda-reader/tree/main/docs/plugins/examples/json-recovery),
which recovers article HTML stored in a page's JSON script element. The
[security model](../security/) describes runtime limits and available permissions.
