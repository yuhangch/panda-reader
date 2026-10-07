# Panda Reader

[![Website](https://img.shields.io/badge/Website-GitHub%20Pages-4b6b56)](https://yuhangch.github.io/panda-reader/)
[![Release](https://github.com/yuhangch/panda-reader/actions/workflows/release.yml/badge.svg)](https://github.com/yuhangch/panda-reader/actions/workflows/release.yml)
[![GPUI Kit](https://img.shields.io/badge/UI-GPUI%20Kit-0A7EA4)](https://gpui-kit.com/)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](#installation)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Panda Reader doesn't come with a grand pitch. It isn't the most minimalist reader, the fastest, or an AI showcase. It's simply an RSS reader I wanted for myself, shaped around my own needs as I go. No product can suit everyone. Contributions are welcome—code, ideas, and even a few AI tokens :)—and so are forks made to fit your own reading habits. Panda Reader can be a starting point, and I think that has value too.

Surrounded by algorithmic feeds and an endless churn of updates, I found myself wanting to go back to RSS. News is part of it, but I'm here just as much for independent blogs: ordinary lives, without a big hook, told simply and honestly. These days, choosing an RSS reader can take about as much time as making one yourself. Making my own has one unbeatable advantage: I can change it however I like. That's how Panda Reader began.

Under the hood, Panda Reader is a desktop RSS reader built with [GPUI](https://www.gpui.rs/) and [gpui-kit](https://gpui-kit.com/). Choose a **Local** library managed by Panda Reader, or connect a **Provider** to synchronize subscriptions and reading state with a supported feed service. Local and provider libraries are stored separately; switching the selected source does not delete the other library.

## Features

- Three-column layout: feeds / article list / reader
- Local subscription management with OPML import/export and offline article storage
- Provider sync with [Miniflux](https://miniflux.app/) and [FreshRSS](https://freshrss.org/) ([setup](https://yuhangch.github.io/panda-reader/docs/providers/))
- Unread, starred, and read-later scopes; provider read/star state synchronization
- Full-text extraction from the original page (manual or automatic)
- Local article plugins for publisher-specific extraction and cleanup ([plugin guide](https://yuhangch.github.io/panda-reader/docs/plugins/))
- Extensible article translation with pluggable providers
- Theme presets and a multilingual interface

## Installation

Download the latest Windows, macOS, or Linux build from [Releases](../../releases). Checksums are included; macOS releases include a note with their signing status.

After installing the Windows per-user setup, macOS app, or Linux AppImage, Panda Reader checks for stable updates and can install them in the background. The portable Windows ZIP and Linux tarball are updated manually from Releases. The first update from an older build also requires installing the new package once.

## Development

Run the app locally with:

```bash
cargo run -p panda-reader
```

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow. Report vulnerabilities privately using [GitHub Security Advisories](../../security/advisories/new); see [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE). Bundled fonts (Inter, Source Sans 3, Source Serif 4, and Noto Sans SC) are under the SIL Open Font License. Their license texts and the TTY7 icon license are included in `apps/panda-reader/assets/`.
The documentation site embeds [IBM Plex Serif](https://github.com/IBM/plex) for headings and [iA Writer Quattro](https://ia.net/writer) for body text; their license files are included in `docs/public/fonts/`.

The generated [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) lists Rust dependency licenses and is included in release packages. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for bundled asset attributions.

## Acknowledgements

Special thanks to [l0ng-ai](https://github.com/l0ng-ai) for creating [TTY7](https://github.com/l0ng-ai/tty7) and [Papr](https://github.com/l0ng-ai/papr). Panda Reader draws inspiration from TTY7's themes and packaging, and Papr's feed parsing, article extraction, and store design.
