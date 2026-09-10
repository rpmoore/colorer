---
type: concept
title: Show Command
description: `colorer show <id>` re-discovers across all backends every call, has no cached state from `list`, and reads sysfs attribute files directly for current color/brightness.
resource: colorer/cli
tags: [cli, device, testability]
---

# Show Command

`src/commands/show.rs::run_show(backends, id)` implements `colorer show <id>`. It re-runs discovery across every given backend on each call — no caching between a `list` and a later `show` invocation, matching `list`'s own behavior (`docs/knowledge/device/backend-and-id.md`).

## Lookup and partial-failure handling

Devices from all backends are pooled, then searched by `id` equality. No match returns `DeviceError::NotFound { id }`.

A backend whose `discover()` errors is skipped (not propagated), same tolerance as `list`'s merge (`docs/knowledge/device/sysfs-classification.md`). Because `show`'s `Ok`/`Err` contract is a single formatted string or a `NotFound` error — unlike `list`, which always returns `Ok` and can embed a warning line in its output — a failing backend is reported via `eprintln!("warning: backend discovery failed: {err}")` at discovery time instead of being embedded in the returned string.

## Current color/brightness

HID-sourced devices always show `"unknown"` — no read path exists yet (a later milestone's concern). Sysfs-sourced devices read one attribute file directly under `DeviceInfo::path`, chosen by `capability`:

- `MultiColor` → `multi_intensity`
- `VendorColor` → `color`
- `SingleColor` / `Unknown` → `brightness`

Missing, unreadable, or empty content all fall back to `"unknown"` rather than erroring — `show` is a best-effort, read-only detail command.

## Shared formatting with `list`

`DeviceCapability`'s string representation (`"unknown"` / `"single-color"` / `"multi-color"` / `"vendor-color"`) is a single `impl fmt::Display` in `src/device/mod.rs`, used by both `list`'s table and `show`'s detail output — added when `show` was introduced so a fourth capability variant wouldn't require updating two separate match statements.
