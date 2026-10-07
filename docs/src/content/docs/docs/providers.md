---
title: Provider setup
description: Choose a local library or connect Panda Reader to Miniflux or FreshRSS.
---

Panda Reader has two library sources, selectable in **Settings → General → Subscription source**:

- **Local**: Panda Reader owns the subscription list and local article cache. Feeds can be added, edited, removed, refreshed, and transferred with OPML without a server.
- **Provider**: subscriptions come from the selected service. Panda Reader keeps a separate local cache and synchronizes supported reading state. Changing providers switches to that provider's separate library; it does not merge or delete libraries.

## Miniflux

Choose Miniflux, enter the instance URL and an API token, then connect. Categories are shown as folders. Feed subscription changes, refresh, OPML, read state, and starred state use Miniflux's REST API.

## FreshRSS

Choose FreshRSS, enter the instance URL, username, and API password, then connect. The API password is configured in FreshRSS account settings. Panda Reader uses the Google Reader-compatible API. Subscription listing/add/remove, article pagination, read/star state, and OPML are available through that API. FreshRSS's separate Fever API is not used.

Provider settings are stored separately per provider. A disconnected provider's cached library remains available when selected, but server operations require reconnecting. Different providers may contain the same feed URL without sharing articles or reading state.

## Data migration

On upgrade, feeds previously marked as local remain in the Local library. Existing Miniflux feeds, cached articles, pending state changes, and account state move into the Miniflux Provider library. If the existing installation has Miniflux credentials and no saved library selection, the app opens the Miniflux library by default. Existing data is retained when switching sources.
