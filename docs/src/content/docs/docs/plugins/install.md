---
title: Install and update plugins
description: Install a plugin from a folder, archive, URL, or the community catalog.
---

## Install a plugin you have

Open **Settings → Plugins** and choose an installation source. Panda Reader accepts a plugin
directory, a ZIP archive containing one plugin, a local path, or an HTTPS URL to a ZIP archive.
The app validates and copies the plugin into its user data directory; it does not run it from the
source folder.

User plugins cannot use the reserved `builtin.*` IDs. ZIP archives are limited to 8 MiB and 64
entries. Remote imports must use HTTPS and have an 8 MiB download limit.

## Install from the community catalog

The **Community plugins** list is backed by the repository's
[community catalog](https://github.com/yuhangch/panda-reader/tree/main/plugins/community). Choose
a plugin to install it. When an update is available, Panda Reader checks the downloaded files'
SHA-256 hashes and verifies that their manifest matches the catalog before replacing the installed
copy. Updates are offered for you to install; they are not applied silently.

## Maintain the catalog

Community plugins are maintained under `plugins/community/`. After editing a manifest or payload,
run the catalog generator locally:

```sh
python scripts/generate_community_plugin_index.py
```

To check whether the committed catalog is current without changing it:

```sh
python scripts/generate_community_plugin_index.py --check
```

See the repository's [community plugin directory](https://github.com/yuhangch/panda-reader/tree/main/plugins/community)
for examples and the generator source.
