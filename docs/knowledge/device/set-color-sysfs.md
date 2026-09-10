---
type: concept
title: Sysfs Set-Color Path
description: "`SysfsBackend::set_color` (via `ColorWriter`) writes color to `MultiColor` and `VendorColor` LED-class devices; udev rule and real-hardware verification are deferred pending a real target device on the dev machine."
resource: colorer/device
tags: [device, sysfs, testability, open-question]
---

# Sysfs Set-Color Path

`impl ColorWriter for SysfsBackend` (`src/device/sysfs.rs:299`) implements `colorer set`'s write path for sysfs LED-class devices. Its body is `set_color_core` (`src/device/sysfs.rs:259`), a dependency-injected core (fresh `discover` closure, `Rgb`) mirroring `hid.rs`'s `set_color_impl` — kept free of a hardcoded `self.discover()` call so revalidation can be exercised with fakes in tests without real `/sys` access.

## `set_color_core`'s steps

1. `discover()` to resolve `id` → `original: DeviceInfo` (`NotFound` if absent).
2. `DeviceCapability::SingleColor` (fixed-color) devices are rejected immediately as `Unsupported`, before revalidating or touching any file — there is no color to change, only brightness, which this plan doesn't expose as a command.
3. `discover()` again and re-find `id`; compare `capability` against `original` (`identity_matches`, `src/device/sysfs.rs:246`) — `DeviceGone` if absent now, `IdentityMismatch` if the directory's attribute shape looks different. Sysfs devices carry no vendor/product id (unlike HID), so capability is the only available identity proxy — a device whose `multi_index` channel *count* changed while `capability` stays `MultiColor` is not caught here; see Known limitations below.
4. Dispatch on `original.capability`: `write_multi_color` for `MultiColor`, `write_vendor_color` for `VendorColor`.

## `write_multi_color` (`src/device/sysfs.rs:214`)

Reads `multi_index` fresh at write time (never assumes an order) and parses it via `parse_channel_order` (`src/device/sysfs.rs:147`), which also **validates** the parsed set is exactly `{red, green, blue}` once each — a malformed or non-RGB channel list (missing/duplicate channel) is rejected with `DeviceError::Io` rather than silently zipped into wrong/duplicate channel writes. Reads `multi_max_intensity` (`parse_intensity_list`, `src/device/sysfs.rs:170`, length-checked against the channel count) and scales the incoming 8-bit `Rgb` via `scale_multi_intensity` (`src/device/sysfs.rs:133`, pure, unit-tested directly) — a `u64` intermediate product guards against overflow from an untrusted/corrupt ceiling value before dividing by 255. Writes the scaled values to `multi_intensity` in `multi_index`'s order, then also writes `max_brightness`'s value into `brightness` — a `multi_intensity` write alone is invisible while `brightness` is low, since `led_brightness = brightness * multi_intensity / max_brightness`.

## `write_vendor_color` (`src/device/sysfs.rs:235`)

Writes `color` as an `aabbcc`-format lowercase hex string directly to the `color` file — the `hid-ite8291r3`-style vendor attribute `SysfsBackend::discover` already probes for and records as `DeviceCapability::VendorColor` (see `docs/knowledge/device/sysfs-classification.md`).

## Error mapping

`map_io_error` (`src/device/sysfs.rs:198`) distinguishes a `PermissionDenied`-kind `io::Error` (mapped to `DeviceError::PermissionDenied`, surfacing "see udev setup") from every other I/O failure, which becomes `DeviceError::Io`. Malformed attribute *content* (bad `multi_index` token, non-numeric `multi_max_intensity`, channel-count mismatch) also surfaces as `DeviceError::Io` — there's no dedicated parse-error variant in `DeviceError`; adding one was judged out of scope for this section (see code review at `docs/plans/implementation/code_review/section-06-review.md`).

## Known limitations

- **Coarse revalidation**: `identity_matches` only compares `DeviceCapability`, not full attribute shape (e.g. channel count/names). A device that keeps `MultiColor` but changes its `multi_index` channel count between the two `discover()` calls is not caught at the revalidation step — it surfaces downstream as a generic `Io` error from `write_multi_color`'s channel-set validation instead of `IdentityMismatch`. Documented tradeoff, not an oversight (no vendor/product id exists on sysfs to compare, unlike HID).
- **No retry on write**: unlike `hid.rs`'s `set_color_impl`, `set_color_core` does not retry a failing write. `hid.rs`'s retry exists specifically to absorb the window right after `udevadm trigger` where rules haven't applied yet — relevant here too once the deferred udev rule (below) lands, but not added yet since there's no real device to validate the retry against.

## Deferred: udev rule and hardware verification

This section's milestone (M5) is conditional on a real `MultiColor`/`VendorColor` sysfs LED device being present on the dev machine — the plan explicitly forbids fabricating a target. As of this implementation, `/sys/class/leds` here only has `SingleColor` fixed-color LEDs (keyboard lock indicators, network link LEDs), so:

- `packaging/udev/71-colorer.rules`'s sysfs-attribute addition (concrete `KERNEL==` name + `chgrp`/`chmod` attribute list) is **not implemented** — it needs a real target device to identify the attribute list and avoid shipping a placeholder VID/PID/path.
- Manual hardware verification (`cargo run -- set <id> ff0000` against real hardware, confirming the `brightness * multi_intensity / max_brightness` interaction) has not been done.

All `set_color`/`scale_multi_intensity` logic above is exercised entirely through `SysfsBackend::new(root)`'s temp-directory fixture seam — no real hardware or `/sys` access is required for `cargo test` to pass.
