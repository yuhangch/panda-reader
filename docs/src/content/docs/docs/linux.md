---
title: Linux support
description: Linux release compatibility and graphics requirements for Panda Reader.
---

Panda Reader's Linux releases target **x86_64 glibc-based distributions**. The current AppImage and tarball are built on Ubuntu 22.04 and require **glibc 2.35 or newer**. Alpine and other musl-based systems are not supported by these builds.

Panda Reader uses GPUI for its Linux interface. A working desktop session and compatible system graphics driver are required; the AppImage does not bundle GPU drivers. Keep your distribution's graphics drivers and Vulkan/OpenGL runtime up to date. A software-rendered or virtual display may not provide a graphics backend that can create a window.

If startup fails, launch the AppImage from a terminal to see graphics initialization errors, then check that the active graphics driver supports the current desktop session (X11 or Wayland).
