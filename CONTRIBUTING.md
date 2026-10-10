# Contributing

Thanks for helping improve Panda Reader. Issues and pull requests are welcome.

## Development checks

Use the stable Rust toolchain and run these commands before submitting a change:

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo check --workspace --locked
```

The `Release` GitHub Actions workflow builds and packages Windows, macOS, and Linux artifacts when a `v*` tag is pushed; run the checks above locally before opening a pull request. Community plugin catalog consistency is checked locally with `python scripts/generate_community_plugin_index.py --check`; regenerate it after plugin changes with `python scripts/generate_community_plugin_index.py`. On the maintainer's Windows machine, keep `.cargo/config.toml` unchanged because it places linker output on a local disk.

## Pull requests

- Describe the user-visible change and any migration or provider behavior.
- Include focused tests for data migrations, sync behavior, and error handling when applicable.
- Do not include credentials, personal feed exports, or private data in commits or issue attachments.
- Keep provider-specific behavior behind the provider abstraction and describe unsupported capabilities in the UI/docs.

## Adding a provider

Implement the provider in `crates/panda-providers`, document supported operations and limitations, and add mock HTTP tests for authentication, pagination, state updates, feed changes, and API errors. Ensure its data is stored in a distinct workspace.

## Adding a community article plugin

Use the repository [Panda Reader plugin skill](.agents/skills/panda-reader-plugin/SKILL.md) when authoring a site-specific rule, and see the [plugin documentation](https://yuhangch.github.io/panda-reader/docs/plugins/) for the format and local testing workflow. Community plugins live under `plugins/community/community.<site>/`. Bump the plugin version when its behavior changes, then regenerate and locally check the catalog with `python scripts/generate_community_plugin_index.py` and `python scripts/generate_community_plugin_index.py --check`.
