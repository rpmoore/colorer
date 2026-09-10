---
type: concept
title: Set Command
description: "`colorer set <id> <color>` parses the color, then tries each `ColorWriter` in turn for one that recognizes `id`; first non-NotFound result wins."
resource: colorer/cli
tags: [cli, device]
---

# Set Command

`src/commands/set.rs::run_set(writers, id, color_input)` implements `colorer set <id> <color>`. It parses `color_input` via `color::parse_color` first — an invalid color string returns `SetError::InvalidColor` before any writer is touched (no discovery, no device I/O attempted).

## Dispatch across writers

Unlike `list`/`show`, which pool results from every `DeviceBackend`, `set` dispatches to a `ColorWriter` (`src/device/mod.rs`) — a separate, smaller trait for the write capability, since not every backend supports writes and each backend's write path has different privilege/protocol requirements than its (always-unprivileged) discovery path. `main.rs::color_writers()` currently returns only `HidBackend`; sysfs write support lands in section-06.

`run_set` tries each writer's `set_color(id, color)` in order. Ids are backend-scoped by their `hid-`/`sysfs-` prefix (`make_id`, `docs/knowledge/device/backend-and-id.md`), so in practice only one writer is ever expected to recognize a given `id` — the loop tolerates trying more than one anyway: a `NotFound` from one writer falls through to the next, while any other error (or success) returns immediately. If every writer reports `NotFound`, that's what's returned.

## Error surfacing

`SetError` wraps either `ColorParseError` or `DeviceError`, both of which already have a `Display` impl with a clear message — `DeviceError::PermissionDenied`'s message specifically includes "see udev setup" (`src/device/mod.rs`), so `main.rs`'s generic `eprintln!("error: {err}")` path needs no special-casing for that case.
