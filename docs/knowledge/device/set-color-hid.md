---
type: concept
title: HID Set-Color Path
description: "`HidBackend::set_color` (via `ColorWriter`) is fully built and tested but returns `Unsupported` for every real device today — `IMPLEMENTED_PROTOCOLS` is intentionally empty until a real target device is identified."
resource: colorer/device
tags: [device, hid, testability, open-question]
---

# HID Set-Color Path

`impl ColorWriter for HidBackend` (`src/device/hid.rs`) implements `colorer set`'s write path for HID devices. Its body is `set_color_impl`, a dependency-injected core (fresh `discover` closure, protocol resolver, `Rgb`, transport-open closure, retry attempt count, delay closure) kept free of real `hidapi` calls so it's exercisable with fakes in tests without hardware.

## No real device is implemented yet

`IMPLEMENTED_PROTOCOLS: &[(u16, u16, ReportBuilder)]` is **empty**. Section-05's precondition (a real target device identified, with a known/reverse-engineered HID report protocol) was not met when this section was implemented, so `set_color` currently returns `DeviceError::Unsupported` for every real device, regardless of whether it's in `vendors.rs`'s known-RGB-vendor allowlist used by `list` — allowlist membership for `list` and protocol support for `set` are independent gates; `implemented_protocol()` is the only place that decides the latter. Adding a real device is a matter of: reverse-engineering its report format, writing a `ReportBuilder` (`fn(&Rgb) -> Vec<u8>`) for it, and adding a `(vendor_id, product_id, builder)` row here — none of the surrounding machinery needs to change.

## `set_color_impl`'s steps

1. `discover()` to resolve `id` → `original: DeviceInfo` (`NotFound` if absent).
2. `implemented_protocol(original.vendor_id, original.product_id)` — `Unsupported` if none (cheap check, run before the second `discover` below to avoid a second enumeration for devices that could never be written to anyway).
3. `discover()` again and re-find `id`; compare `vendor_id`/`product_id`/`interface_number` against `original` (`identity_matches`) — `DeviceGone` if absent now, `IdentityMismatch` if a different device now occupies the id.
4. Build the report via the resolved `ReportBuilder`, then `retry_with_delay` (fixed `RETRY_ATTEMPTS = 3`, injectable `delay`) opening a transport and calling `HidTransport::write_report` each attempt.

**Revalidation's actual scope — a known limitation, not a solved problem:** both `discover()` calls happen within a single `set` invocation, microseconds apart, so this only catches a device swap racing *within that window* (e.g. another process replugging hardware mid-call). It does **not** protect against the coarser, more realistic race the original plan's "Device identity note" was concerned with: a device seen in an earlier `list` invocation gets unplugged and replaced by a different device reusing the same path (and thus the same `id`) before `set` is even run. `set` only receives an opaque `id` — it has no independently-recorded "what `list` last saw at this id" to compare against, so this class of race is structurally uncatchable by the current design. Closing that gap would require `set` to accept and verify caller-supplied identity (e.g. an expected vendor/product id argument), a larger contract change not part of this section. See also the still-unresolved HID-interface-dedup question (`docs/knowledge/device/hid-interface-enumeration.md`) — it remains unresolved for the same reason: no real device to decide it against yet.

## Retry and transport abstraction

`retry_with_delay(attempts, delay, op)` (`src/device/hid.rs`) retries a fallible `FnMut` up to `attempts` times, calling `delay()` between attempts but never after the last one, surfacing the final error once exhausted. `delay` is injected specifically so tests never sleep for real. The retried operation re-opens the transport each attempt (`open_transport(&original)` then `write_report`) — this absorbs the brief window where udev hasn't finished applying rules to a just-plugged device, at the cost of re-running `HidApi::new()` on every attempt in the (currently unreached, since the registry is empty) success path; worth revisiting once a real protocol makes this a hot path.

`HidTransport` (module-private trait, `write_report(&self, report: &[u8])`) abstracts the raw write so `set_color_impl` never touches `hidapi` directly. `RealHidTransport` wraps an opened `hidapi::HidDevice` in production (`open_real_transport`); tests use their own fakes.

## Error mapping

`map_hid_error` (`src/device/hid.rs`) distinguishes `hidapi::HidError::IoError` wrapping a `PermissionDenied`-kind `io::Error` — mapped to `DeviceError::PermissionDenied` — from every other `hidapi` error, which becomes `DeviceError::Hid`. This is best-effort (untested against real hardware, since no real device exists yet) but lets command-layer code print udev-setup guidance specifically for the permission case rather than a generic I/O error.
