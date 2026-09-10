---
type: concept
title: Sysfs LED Classification
description: SysfsBackend classifies /sys/class/leds entries by attribute-file presence, with a deliberate precedence order that keeps vendor-color detection reliable.
resource: colorer/device
tags: [device, sysfs, testability]
---

# Sysfs LED Classification

`src/device/sysfs.rs` implements `SysfsBackend`, the second `DeviceBackend` (alongside `HidBackend`), scanning a configurable root (`/sys/class/leds` in production, a `tempfile` fixture dir in tests — the constructor-parameter seam is `SysfsBackend::new(root)`).

## Classification precedence

For each entry, `discover()` checks attribute files in this order:

1. `multi_intensity` **and** `multi_index` both present → `DeviceCapability::MultiColor`.
2. Exactly one of `multi_intensity`/`multi_index` present (not both) → malformed, skipped with a warning, not fatal to the scan.
3. `color` file present → `DeviceCapability::VendorColor`.
4. `brightness` **and** `max_brightness` present → `DeviceCapability::SingleColor`.
5. None of the above → skipped with a warning.

**The `color` check (step 3) deliberately runs before the `brightness` check (step 4)** — this is the opposite of a naive reading of the plan prose. Real vendor-color drivers (e.g. `hid-ite8291r3`-style) typically expose `brightness`/`max_brightness` *alongside* `color`; checking brightness first would misclassify them as `SingleColor` and silently lose the vendor-color fact that a later milestone (M5/section-06, which writes to `color`) depends on being recorded at discovery time. Pinned by `vendor_color_wins_over_brightness_when_both_present` (`src/device/sysfs.rs`).

`Path::exists()` is used for all these probes, which reports `false` for both a genuinely absent file and one that exists but is unreadable (permission error) — the two cases are not distinguished; both fall through to the same skip/classify paths.

## `DeviceCapability::VendorColor`

Added as a new variant on the existing `DeviceCapability` enum (`src/device/mod.rs`) rather than a separate boolean field, so a single `match` still fully describes a device's discovery-time capability. This is the representation section-06 (M5) reads to determine, from discovery output alone, whether a device exposes the vendor `color` write path — see `docs/plans/sections/section-06-set-sysfs.md`.

## Merge and partial-failure policy in `list`

`run_list` (`src/commands/list.rs`) calls `discover()` on every configured backend independently, collecting `Ok` results and turning any `Err` into a `warning: backend discovery failed: {err}` line rather than propagating it — one backend failing (e.g. `/sys/class/leds` unreadable) never suppresses another backend's results. The warning text carries only the bare `DeviceError` message, not which backend produced it, since `DeviceBackend` carries no name/label; this was flagged as an intentional scope cut during section-03's review, deferred until a section actually needs per-backend diagnostics.
