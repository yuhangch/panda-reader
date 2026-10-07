---
title: Plugin security model
description: Learn what plugins can access and how the host contains failures.
---

Plugins transform article content through a host-controlled API. They do not receive direct
network, filesystem, database, process, or UI access. HTML parsing, URL handling, extraction, and
final sanitization remain under the app's control.

WASM execution is bounded by runtime limits: modules are limited to 4 MiB, guest memory to 16 MiB,
each stage to 10 million units of Wasm fuel, and an invocation to 10,000 host calls. Decoded HTML
is limited to 8 MiB and 100,000 document nodes; JSON requests and responses are limited to 2 MiB
each. Fuel limits executed instructions, not wall-clock time.

Unknown imports, invalid pointers, missing capabilities, and invalid node handles fail the plugin
invocation. A failing plugin's changes are rolled back so the rest of the article pipeline can
continue.
