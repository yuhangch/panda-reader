# Google News URL resolver

This community Wasm plugin converts Google News article wrapper URLs to the
publisher URL before Panda Reader downloads the article body. It is optional;
users who do not subscribe to Google News do not need to install it.

The plugin uses Panda Reader's brokered HTTP operation. Its manifest permits
HTTPS requests to `news.google.com` only. The host enforces the allowlist,
request timeout, four-request budget, and 1 MiB request and response limits. The
plugin cannot open sockets or access the filesystem directly.

## Build

The repository includes `plugin.wasm`, so the plugin can be imported directly.
To rebuild it, install the `wasm32-unknown-unknown` target and run:

```powershell
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\panda-reader-plugin-build"
cargo build --manifest-path plugins/community/community.google-news-resolver/Cargo.toml --target wasm32-unknown-unknown --release
Copy-Item "$env:CARGO_TARGET_DIR/wasm32-unknown-unknown/release/panda_google_news_resolver.wasm" plugins/community/community.google-news-resolver/plugin.wasm
```

The Rust source demonstrates the `resolve_url` stage, bounded host HTTP calls,
Google News response parsing, and returning the publisher URL through the SDK.
