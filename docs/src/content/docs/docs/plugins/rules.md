---
title: Rule plugins
description: Use TOML rules to select and clean article content without writing code.
---

Rules are declarative TOML actions evaluated in file order. Selectors are validated when the
plugin is loaded. Rule plugins can select an article body, remove matching nodes, remove nodes by
exact normalized text, set or remove attributes, and repair lazy-loaded image attributes.

Use `prepare` rules to repair source HTML before extraction. Use `cleanup` rules to remove unwanted
content from the article body after RSS content or extracted web content is available.

The repository includes a small
[rules plugin example](https://github.com/yuhangch/panda-reader/tree/main/docs/plugins/examples/rules)
and a local HTML fixture. The [manifest guide](../manifest/) explains stage and URL matching.
