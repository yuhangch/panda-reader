# Community plugins

Plugins in this directory are community-maintained site rules and examples. Each plugin is an
independent directory that can be imported from Settings → Plugins. The application copies the
plugin into its user data directory; keeping the source here makes review, version history, and
releases straightforward without treating third-party site rules as application code.

[`index.json`](index.json) is the generated community update catalog. It lists each plugin's
stable ID, version, minimum app version, API version, and direct HTTPS URLs with SHA-256 hashes
for installable files. The generator reads each plugin's `manifest.toml` and payload, so edit the
source files, bump the plugin version in its manifest when behavior changes, and run:

```sh
python scripts/generate_community_plugin_index.py
```

The script uses the Python 3.11+ standard library. CI checks that the committed catalog is up to
date. README and fixture files are for maintainers and are not downloaded by the app.

- [`community.sciencenet`](community.sciencenet) removes ScienceNet's duplicate header table and
  standalone reposting notice.
- [`community.ithome`](community.ithome) removes IT Home's standalone ticket-promotion paragraph
  and external-link advertising disclosure.
- [`community.chinanews`](community.chinanews) removes the duplicated article
  header row and page controls from China News Network pages.
- [`community.wechat`](community.wechat) removes the generated QR/share image from WeChat articles.
- [`community.qbitai`](community.qbitai) removes QbitAI's publisher logo from article content.
