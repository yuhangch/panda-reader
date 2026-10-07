---
name: panda-reader-plugin
description: Create a Panda Reader article plugin from a site-specific cleanup or extraction request. Use declarative rules for selectors and exact text; use Wasm only when the transformation needs code.
---

# Create a Panda Reader article plugin

Create an importable Panda Reader plugin for the user's requested publisher or article-page
transformation. Read the [plugin overview](../../../docs/src/content/docs/docs/plugins/index.md)
for links to the current manifest, rule actions, capabilities, and Wasm SDK details. Use the examples in
`docs/plugins/examples/` as working references.

## Choose the smallest plugin type

- Prefer `kind = "rules"` for selecting/removing elements, removing a paragraph by exact normalized
  text, changing attributes, or repairing lazy-loaded images.
- Use `kind = "wasm"` only when a rule cannot express the operation, such as conditional logic or
  recovering article HTML from embedded JSON. Keep HTML parsing and DOM operations in the host.
- Plugins process article HTML only. Do not use them for provider sync, translation, reader display,
  or application settings.

## Create the plugin

1. Use a user-provided page excerpt or fixture to identify the smallest stable selector or exact
   text. If the structure is ambiguous, ask for a sanitized HTML sample rather than guessing a broad
   selector.
2. Create `manifest.toml` with a stable ID, semantic version, `api_version = 1`,
   `min_app_version`, `kind`, exact publisher domain or `*.domain`, stage, priority, and only the
   required capabilities. Use `community.<site>` only when the user asks to contribute it to this
   repository; otherwise use a distinct user plugin ID.
3. For rule plugins, create `rules.toml`. Add concise English comments that explain why each
   site-specific rule exists. Prefer exact normalized paragraph text or narrow publisher-specific
   selectors; avoid broad selectors likely to remove article content.
4. Add a small `fixture.html` demonstrating both the target artifact and nearby content that must
   remain. Include a short README describing the behavior and matching domain.
5. Put a user plugin in a standalone directory the user can import. For a requested community
   contribution, use `plugins/community/community.<site>/`, include it in the community listing,
   and regenerate the catalog with `python scripts/generate_community_plugin_index.py`.

## Validate and deliver

- Validate the manifest and rules with the plugin runner when the Panda Reader workspace is
  available. The runner takes a plugin directory, article URL, title, and local fixture; it does
  not fetch network content. See the [testing guide](../../../docs/src/content/docs/docs/plugins/testing.md).
- For Wasm, use `docs/plugins/examples/json-recovery` and build for
  `wasm32-unknown-unknown`; name the runtime module `plugin.wasm` in the importable plugin folder.
- Keep the deliverable limited to the plugin directory (or a ZIP if the user asks for one). Tell
  the user which directory to choose in Settings → Plugins and note any assumptions about the
  site structure.
- Do not claim a plugin works on live pages unless it was validated against representative HTML.
