# Application source

`main.rs` calls `app::run()`. The source has three module boundaries:

- `app`: startup, data paths, saved preferences, and legacy configuration migration. Configuration types do not depend on GPUI.
- `ui`: window composition, feature state, rendering, and interaction. Theme palettes and UI translations belong here.
- `services`: background execution and application operations. These coordinate the existing store, provider, and translation crates.

The window owns the sidebar, article list, article view, settings, feed editor, status, and command palette. Each feature constructs its own controls, retains its subscriptions, and renders its state. Feature `actions.rs` files implement window methods when an operation coordinates multiple panels or updates state after a service reply.

`ui/article_view` displays articles. `services/articles/content.rs` prepares their HTML off the UI thread; `cache.rs` caches the result. Neither content preparation nor its cache depends on GPUI.

The service dispatcher routes typed commands to feature modules. Worker lifecycle and task execution belong in `services/worker.rs`; store and provider operations belong in the corresponding service module.

The desktop binary has its test harness disabled because of UI macro expansion on Windows. Integration tests under `tests/` load the configuration and service modules directly, including their unit tests, without compiling the window as a test harness. Run them with `cargo test -p panda-reader`. On this machine, set the local `CARGO_TARGET_DIR` documented in the repository's `AGENTS.md` before any build or test.
