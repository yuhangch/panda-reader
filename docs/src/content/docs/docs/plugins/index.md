---
title: Article plugins
description: Extend Panda Reader with small, site-specific article cleanup plugins.
---

Panda Reader plugins help make publisher pages readable when a feed only includes a short
summary or the original page has clutter the reader cannot handle on its own. A plugin can repair
article HTML before extraction or clean the article body afterward.

The host keeps control of HTML parsing, URL handling, extraction, and final sanitization. Plugins
run in a constrained environment without direct network, filesystem, database, process, or UI
access. If one fails, its changes are discarded and the rest of the reading pipeline continues.

## Choose a plugin format

- **Rules** are declarative TOML actions for common cleanup tasks. They are easy to inspect and
  are a good place to start.
- **WASM** plugins can express more involved transformations while using the host's limited
  document API.

Each plugin has a `manifest.toml` and one payload: `rules.toml` or `plugin.wasm`.

## Community plugins

The community directory includes examples for ScienceNet, WeChat, IT Home, QbitAI, and China News.
You can install a community plugin from Settings → Plugins, maintain one in the repository, or
publish your own for users to import.

## Guides

- [Install and update plugins](./install/)
- [Plugin manifest](./manifest/)
- [Rule plugins](./rules/)
- [WASM plugins](./wasm/)
- [Security model](./security/)
- [Test plugins locally](./testing/)

For agent-assisted plugin authoring, see the repository's
[Panda Reader plugin skill](https://github.com/yuhangch/panda-reader/blob/main/.agents/skills/panda-reader-plugin/SKILL.md).
