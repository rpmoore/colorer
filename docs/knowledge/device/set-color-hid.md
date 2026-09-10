---
type: concept
title: HID Set-Color Path
description: "`HidBackend::set_color` (via `ColorWriter`) has one real, hardware-verified protocol entry (Razer Ornata V3, Feature report) in `IMPLEMENTED_PROTOCOLS`; every other device still returns `Unsupported`."
resource: colorer/device
tags: [device, hid, testability, razer]
---

# HID Set-Color Path

`impl ColorWriter for HidBackend` (`src/device/hid.rs`) implements `colorer set`'s write path for HID devices. Its body is `set_color_impl`, a dependency-injected core (fresh `discover` closure, protocol resolver, `Rgb`, transport-open closure, retry attempt count, delay closure) kept free of real `hidapi` calls so it's exercisable with fakes in tests without hardware.

## `IMPLEMENTED_PROTOCOLS`

`&[(u16 vendor_id, u16 product_id, i32 interface_number, ReportKind, ReportBuilder)]`. Keyed by vendor/product **and interface number**, not vendor/product alone: a single physical device like the Ornata V3 enumerates as several `DeviceInfo` rows (one per HID interface — see `hid-interface-enumeration.md`) sharing one vendor/product id, but only one specific interface accepts a given command protocol. Being in `vendors.rs`'s known-RGB-vendor allowlist (used by `list`) does NOT imply an entry here — `list` support and `set` support are independent gates; `implemented_protocol()` is the only place that decides the latter.

**Razer Ornata V3** (`1532:02a1`, interface 2, `hid-0454c261` on the dev machine) is the one real, hardware-confirmed entry, added and verified live against physical hardware. Protocol confirmed against `openrazer/openrazer`'s `razerkbd_driver.c`/`razerchromacommon.c` source (not guessed): `USB_DEVICE_ID_RAZER_ORNATA_V3` dispatches `matrix_effect_static` to `razer_chroma_extended_matrix_effect_static(VARSTORE, BACKLIGHT_LED, rgb)` with `transaction_id = 0x1F`, `report_index = response_index = 0x02` (matches this device's `interface_number`).

## `set_color_impl`'s steps

1. `discover()` to resolve `id` → `original: DeviceInfo` (`NotFound` if absent).
2. `implemented_protocol(original.vendor_id, original.product_id, original.interface_number)` — `Unsupported` if none (cheap check, run before the second `discover` below).
3. `discover()` again and re-find `id`; compare `vendor_id`/`product_id`/`interface_number` against `original` (`identity_matches`) — `DeviceGone` if absent now, `IdentityMismatch` if a different device now occupies the id.
4. Build the report via the resolved `ReportBuilder`, then `retry_with_delay` (fixed `RETRY_ATTEMPTS = 3`, injectable `delay`) opening a transport and dispatching on the resolved `ReportKind`: `Output` calls `HidTransport::write_report` (a plain interrupt report), `Feature` calls `HidTransport::write_feature_report` (a control-transfer Feature report via `HIDIOCSFEATURE`).

**Revalidation's actual scope — a known limitation, not a solved problem:** both `discover()` calls happen within a single `set` invocation, microseconds apart, so this only catches a device swap racing *within that window*. It does **not** protect against a device seen in an earlier `list` invocation being unplugged and replaced by a different device reusing the same path/id before `set` is even run — `set` only receives an opaque `id`, with no independently-recorded "what `list` last saw at this id" to compare against. Closing that gap would need `set` to accept and verify caller-supplied identity, a larger contract change not part of this section.

## `ReportKind`: Output vs. Feature

`HidTransport` (`src/device/hid.rs`) has two write methods because devices disagree on which USB transfer their color protocol uses: `write_report` (interrupt OUT, `hidapi::HidDevice::write`) vs. `write_feature_report` (control transfer, `hidapi::HidDevice::send_feature_report`). Each `IMPLEMENTED_PROTOCOLS` entry carries a `ReportKind` saying which one to use. Razer's Chroma protocol needs `Feature`; `Output` currently has no real entry using it but is kept as part of the stable contract for a future device that does.

## Razer Chroma report framing — the empirically-discovered gotcha

`razer_ornata_v3_static_struct` builds the 90-byte `razer_report` struct exactly as documented upstream (`status(1) transaction_id(1) remaining_packets(2,BE) protocol_type(1) data_size(1) command_class(1) command_id(1) arguments(80) crc(1) reserved(1)`; CRC = XOR of bytes `[2..88)`, mirroring `razer_calculate_crc`). This part matches the openrazer source exactly and was verified against its doc-comment byte captures in the unit test.

**What upstream's C source does *not* need, but this Rust/`hidapi` implementation does:** `razer_ornata_v3_static_report` (the actual `ReportBuilder`) prefixes that 90-byte struct with an explicit `0x00` HID report-ID byte — 91 bytes total. This was not documented anywhere and was found by trial on real hardware: sending the bare 90-byte struct via `hidapi::send_feature_report` returns `Ok`, but reading it back via `get_feature_report` shows the device's `status` byte stuck at `0x00` (never processed) and the keyboard's color doesn't change. Prefixing the `0x00` report-ID byte gets `status = 0x02` (success) back and the color actually updates. The reason upstream's kernel driver doesn't need this: it issues a raw `usb_control_msg` directly, while `hidapi`'s Linux Feature-report calls go through `hidraw`'s `HIDIOCSFEATURE`/`HIDIOCGFEATURE` ioctls, which apparently expect the report-ID byte as an explicit buffer prefix even when the device's descriptor has no `Report ID` tag (report ID 0, unnumbered).

If a future device's protocol is added and also uses `ReportKind::Feature`, re-verify this framing empirically for that device rather than assuming the same prefix behavior — it was discovered by testing against one specific device/driver-backend combination, not derived from a documented `hidapi` contract.

## Retry and transport abstraction

`retry_with_delay(attempts, delay, op)` (`src/device/hid.rs`) retries a fallible `FnMut` up to `attempts` times, calling `delay()` between attempts but never after the last one, surfacing the final error once exhausted. `delay` is injected specifically so tests never sleep for real. The retried operation re-opens the transport each attempt (`open_transport(&original)` then the `ReportKind`-appropriate write) — this absorbs the brief window where udev hasn't finished applying rules to a just-plugged device.

## Error mapping

`map_hid_error` (`src/device/hid.rs`) distinguishes `hidapi::HidError::IoError` wrapping a `PermissionDenied`-kind `io::Error` — mapped to `DeviceError::PermissionDenied` — from every other `hidapi` error, which becomes `DeviceError::Hid`. This lets command-layer code print udev-setup guidance specifically for the permission case rather than a generic I/O error.

## Known conflict risk: other RGB control software

Nothing else was found holding `/dev/hidraw11` open or running as a service when the Ornata V3 protocol was verified (checked `ps`/`fuser`/`lsof`/`systemctl` — see the session that added this entry). If a userspace RGB daemon that also speaks Razer's protocol (e.g. OpenRGB, `openrazer-daemon`) is running, it can fight over device state with `colorer set` — neither locks the device, and this plan has no concurrent-access coordination (see the sysfs backend's equivalent scope note).
