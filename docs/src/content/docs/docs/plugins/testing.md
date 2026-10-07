---
title: Test plugins locally
description: Run a plugin against an HTML fixture without making network requests.
---

The repository includes a fixture runner for exercising plugins against local HTML. It processes
the fixture only and does not fetch the article URL:

```sh
cargo run -p panda-plugins --bin panda-plugin-runner -- \
  ./plugins https://example.com/articles/42 "Example title" \
  docs/plugins/examples/rules/fixture.html
```

To use the included selector rules, set the plugin directory to
`docs/plugins/examples/rules` instead of `./plugins`.

Processed HTML is written to standard output and diagnostics to standard error. Add fixtures for
publisher page variations that could otherwise break extraction or cleanup.
