---
type: concept
title: HID Set-Color Path
description: "`HidBackend::set_color` (via `ColorWriter`) has four real, hardware-verified `IMPLEMENTED_PROTOCOLS` entries (Razer Ornata V3; Razer Naga X; Razer Tartarus Pro; Gigabyte RGB Fusion 2's CPU strip + IO Cover + Chipset Accent); every other device still returns `Unsupported`."
resource: colorer/device
tags: [device, hid, testability, razer, gigabyte]
---

# HID Set-Color Path

`impl ColorWriter for HidBackend` (`src/device/hid.rs`) implements `colorer set`'s write path for HID devices. Its body is `set_color_impl`, a dependency-injected core (fresh `discover` closure, protocol resolver, `Rgb`, transport-open closure, retry attempt count, delay closure) kept free of real `hidapi` calls so it's exercisable with fakes in tests without hardware.

## `IMPLEMENTED_PROTOCOLS`

`&[(u16 vendor_id, u16 product_id, i32 interface_number, ReportKind, ReportBuilder)]`. Keyed by vendor/product **and interface number**, not vendor/product alone: a single physical device like the Ornata V3 enumerates as several `DeviceInfo` rows (one per HID interface — see `hid-interface-enumeration.md`) sharing one vendor/product id, but only one specific interface accepts a given command protocol. Being in `vendors.rs`'s known-RGB-vendor allowlist (used by `list`) does NOT imply an entry here — `list` support and `set` support are independent gates; `implemented_protocol()` is the only place that decides the latter.

**Razer Ornata V3** (`1532:02a1`, interface 2, `hid-0454c261` on the dev machine): added and verified live against physical hardware. Protocol confirmed against `openrazer/openrazer`'s `razerkbd_driver.c`/`razerchromacommon.c` source (not guessed): `USB_DEVICE_ID_RAZER_ORNATA_V3` dispatches `matrix_effect_static` to `razer_chroma_extended_matrix_effect_static(VARSTORE, BACKLIGHT_LED, rgb)` with `transaction_id = 0x1F`, `report_index = response_index = 0x02` (matches this device's `interface_number`).

**Razer Naga X** (`1532:0096`, interface 3, `hid-a40f6985` on the dev machine): added and verified live. Confirmed against `openrazer/openrazer`'s `razermouse_driver.c`: `USB_DEVICE_ID_RAZER_NAGA_X` in `matrix_effect_static_common`'s dispatch also routes to `razer_chroma_extended_matrix_effect_static` (same `transaction_id = 0x1F`), so `razer_naga_x_static_report` shares `razer_chroma_extended_matrix_effect_static_struct` — the same 90-byte struct builder and `0x00`-prefix framing quirk — with the Ornata V3, factored out once a second real user of that shape existed. This mouse has two independent zones (`SCROLL_WHEEL_LED`, `LEFT_SIDE_LED`; it has no logo/backlight zone), both set to the same color in one `set_color` call since this CLI has no per-zone addressing — two reports sent per call, one per zone. Permission note: on the dev machine, this device's `hidraw` nodes did not pick up `uaccess` automatically the way the Ornata V3's did (unrelated system Razer udev rules apparently don't cover every product id, or don't grant `uaccess` at all) — needed `packaging/udev/71-colorer.rules` installed for real (`udevadm control --reload-rules && udevadm trigger`, not just a live `trigger` or a replug) before `set` could open the device.

**Razer Tartarus Pro** (`1532:0244`, interface 2, `hid-a80f6fd1` on the dev machine): added and verified live. Confirmed against `openrazer/openrazer`'s `razerkbd_driver.c`: `USB_DEVICE_ID_RAZER_TARTARUS_PRO` shares the exact same `matrix_effect_static` `case` block as the Ornata V3 (`BACKLIGHT_LED`, `transaction_id = 0x1F`), so `razer_tartarus_pro_static_report` is the same struct shape on a different device/interface — no new protocol details to reverse-engineer, just a table entry and live confirmation.

**Gigabyte RGB Fusion 2 onboard controller** (`048d:5711`, interface 1, `hid-0854c8ad` on the dev machine): added and verified live. See "Gigabyte RGB Fusion 2: two different control paths on one chip" below for the full protocol and its scope caveats — drives 3 of this board's 6 zones (CPU strip, motherboard-logo "IO Cover", "Chipset Accent"), on one specific motherboard model, unlike Razer's whole-device static color.

## `set_color_impl`'s steps

1. `discover()` to resolve `id` → `original: DeviceInfo` (`NotFound` if absent).
2. `implemented_protocol(original.vendor_id, original.product_id, original.interface_number)` — `Unsupported` if none (cheap check, run before the second `discover` below).
3. `discover()` again and re-find `id`; compare `vendor_id`/`product_id`/`interface_number` against `original` (`identity_matches`) — `DeviceGone` if absent now, `IdentityMismatch` if a different device now occupies the id.
4. Build the report *sequence* via the resolved `ReportBuilder` (`fn(&Rgb) -> Vec<Vec<u8>>` — most protocols are one report, but e.g. Gigabyte's is 7), then `retry_with_delay` (fixed `RETRY_ATTEMPTS = 3`, injectable `delay`, retries the whole sequence from the start) opening a transport and writing each report in order, dispatching on the resolved `ReportKind`: `Output` calls `HidTransport::write_report` (a plain interrupt report), `Feature` calls `HidTransport::write_feature_report` (a control-transfer Feature report via `HIDIOCSFEATURE`).

**Revalidation's actual scope — a known limitation, not a solved problem:** both `discover()` calls happen within a single `set` invocation, microseconds apart, so this only catches a device swap racing *within that window*. It does **not** protect against a device seen in an earlier `list` invocation being unplugged and replaced by a different device reusing the same path/id before `set` is even run — `set` only receives an opaque `id`, with no independently-recorded "what `list` last saw at this id" to compare against. Closing that gap would need `set` to accept and verify caller-supplied identity, a larger contract change not part of this section.

## `ReportKind`: Output vs. Feature

`HidTransport` (`src/device/hid.rs`) has two write methods because devices disagree on which USB transfer their color protocol uses: `write_report` (interrupt OUT, `hidapi::HidDevice::write`) vs. `write_feature_report` (control transfer, `hidapi::HidDevice::send_feature_report`). Each `IMPLEMENTED_PROTOCOLS` entry carries a `ReportKind` saying which one to use. Razer's Chroma protocol needs `Feature`; `Output` currently has no real entry using it but is kept as part of the stable contract for a future device that does.

## Razer Chroma report framing — the empirically-discovered gotcha

`razer_ornata_v3_static_struct` builds the 90-byte `razer_report` struct exactly as documented upstream (`status(1) transaction_id(1) remaining_packets(2,BE) protocol_type(1) data_size(1) command_class(1) command_id(1) arguments(80) crc(1) reserved(1)`; CRC = XOR of bytes `[2..88)`, mirroring `razer_calculate_crc`). This part matches the openrazer source exactly and was verified against its doc-comment byte captures in the unit test.

**What upstream's C source does *not* need, but this Rust/`hidapi` implementation does:** `razer_ornata_v3_static_report` (the actual `ReportBuilder`) prefixes that 90-byte struct with an explicit `0x00` HID report-ID byte — 91 bytes total. This was not documented anywhere and was found by trial on real hardware: sending the bare 90-byte struct via `hidapi::send_feature_report` returns `Ok`, but reading it back via `get_feature_report` shows the device's `status` byte stuck at `0x00` (never processed) and the keyboard's color doesn't change. Prefixing the `0x00` report-ID byte gets `status = 0x02` (success) back and the color actually updates. The reason upstream's kernel driver doesn't need this: it issues a raw `usb_control_msg` directly, while `hidapi`'s Linux Feature-report calls go through `hidraw`'s `HIDIOCSFEATURE`/`HIDIOCGFEATURE` ioctls, which apparently expect the report-ID byte as an explicit buffer prefix even when the device's descriptor has no `Report ID` tag (report ID 0, unnumbered).

If a future device's protocol is added and also uses `ReportKind::Feature`, re-verify this framing empirically for that device rather than assuming the same prefix behavior — it was discovered by testing against one specific device/driver-backend combination, not derived from a documented `hidapi` contract.

## Gigabyte RGB Fusion 2: two different control paths on one chip

`gigabyte_fusion2_static_report` (`src/device/hid.rs`) targets **3 of 6 zones on one specific motherboard model** — Gigabyte X870E AORUS PRO. Per `OpenRGB`'s `GigabyteFusion2USB_Devices.cpp`, this board's `it5711_11_device` layout has 6 zones: 3 "Linear" Gen2 ARGB strips (`ARGB_V2_1/2/3`, headers `HDR_D_LED1/2/3`) + 3 "Single" fixed LEDs (`LED_C`, `IO Cover`, `Chipset Accent`). Driven, all confirmed live:
- **`HDR_D_LED2` / "ARGB_V2_2"** — the CPU-area ARGB strip (48 LEDs), via the Gen2 "Direct" addressable-color path.
- **`IO Cover` (`LED10`)** — this board's motherboard-logo LED, via the simple fixed-color "effect" path.
- **`Chipset Accent` (`LED11`)** — same effect path.

Not driven: `ARGB_V2_1`/`ARGB_V2_3` (a live Gen2 scan found 0 LEDs on both — nothing physically connected on this board), and `LED_C` (tried, abandoned — see below). Case fans and any other board lighting are untouched and were confirmed separately unresponsive to *every* zone/header this chip exposes, so are likely wired through a different controller entirely, not just an unimplemented zone here.

### The CPU strip: Gen2 addressable, not a simple effect command

**First attempt failed — a wrong protocol guess, not a framing bug.** The initial implementation used the generic `PktEffect`/`EFFECT_STATIC` "hardware effect" command (`OpenRGB`'s comment: "Motherboard LEDs always use effect mode") broadcast across all 11 possible zones. It sent successfully (report ID `0xCC`, 64-byte Feature reports, no extra framing prefix needed — unlike Razer, since this device numbers its reports) but produced **zero visible change** on real hardware, across three separate attempts (broadcast form, per-zone loop, and replicating `OpenRGB`'s full constructor init sequence). The real cause: this board's `strip_detect` flag (read back as `0x01` from the info command, offset 3 of `IT8297Report`) means its CPU header is a **Gen2 addressable strip**, controlled by an entirely different "Direct" per-LED-color path (`SetStripColors`/`PktRGB`), not the fixed-effect command.

**What actually works, confirmed live:**
1. **Scan/detect** (`OpenRGB`'s `ScanGen2Strips`, not reimplemented here — see below): send `SendCCReport(GEN2_LED_BASE_SCAN + delta[slot], 0, 0)` (`delta = [4,5,0,1]` per slot 0-3), wait 700ms, send the paired info command (`scan_cmd + 2`), then `get_feature_report` and parse `buf[1]` = segment count, `buf[2..]` = little-endian 16-bit LED counts per segment. Run once against this board's slot 1 (`HDR_D_LED2`): **48 LEDs** detected.
2. **Disable the header's built-in effect** (command `0x32`, one byte argument = a bitmask of which headers to disable) so the firmware stops overriding host-set colors with its own animation.
3. **Write LED colors** via `PktRGB`: report byte 1 = header (`HDR_D_LED2_ARGB = 0x59`), bytes 2-3 = little-endian byte offset into the LED array, byte 4 = byte count for this chunk (`leds_in_chunk * 3`), bytes 5+ = 3 bytes per LED. Chunked at 19 LEDs/packet (`sizeof(leds[19])` in `OpenRGB`'s struct) — 48 LEDs needs 3 packets (19+19+10).
4. **Channel byte order is per-board-calibrated, not fixed RGB or BGR.** `OpenRGB` reads this from the device's own `cal_strip1` calibration register (part of the same info-command response) and decodes it via a byte-order lookup table. This board's `cal_strip1` decoded to `"GRB"` — verified by writing pure red (`0xFF0000`) and confirming the strip actually showed red, not green. **Hardcoded for this board, not read dynamically**: `HidTransport` has no read-back capability (`ReportBuilder` is write-only, `fn(&Rgb) -> Vec<Vec<u8>>`), and Gigabyte calibration is a per-board-*model* wiring fact (fixed at manufacturing for a given PCB), not something expected to vary unit-to-unit for the same model.
5. **Apply** (`SendCCReport(0x28, 0xFF, 0x07)`, the `product_id == 0x5711`-specific "fast apply" form).

### IO Cover / Chipset Accent: the "effect" path, done correctly this time

Same `PktEffect`/`EFFECT_STATIC` command the failed CPU-strip attempt used — it's the *right* mechanism for these two single-LED zones, just the wrong one for a Gen2 strip. Header byte is `0x20 + led` for `led < 8`, `0x90 + (led - 8)` otherwise (`IO Cover` = `LED10` = index 9 → header `0x91`; `Chipset Accent` = `LED11` = index 10 → header `0x92`); `zone0` = `1 << led`. Color is packed `[b, g, r]` at offsets 14-16 (`RGBToBGRColor`, the packing the original failed CPU-strip attempt also used — correct for *this* path, just not for the CPU strip's Gen2/Direct path). Confirmed by sending each zone a distinct color and observing them change independently, and by then sending a shared color to all three zones in one real `colorer set` call and observing all three update together.

**`LED_C` (`led = 4`) was tried and abandoned.** Its effect-register write accepted and visually matched once — turned red on the very first attempt, in the same batch that genuinely fixed IO Cover/Chipset Accent — but two subsequent attempts with different colors (cyan, then white, with generous delays) produced no visible change at all. Strong evidence the first "success" was coincidental (the LED was already red from unrelated leftover state, see below) rather than this command reaching a real, populated LED on this board.

### A real behavioral hazard hit while reverse-engineering this, worth remembering for any future Gigabyte work

`SetStripBuiltinEffectState`'s `enable` parameter is inverted from what its name suggests — `enable = true` *re-enables* the firmware's autonomous built-in effect (clears the header's disable-bit); `enable = false` is what hands control to the host. An early debug attempt sent the disable bitmask for *every* header at once (intending to silence a strobing default effect before testing a static color), which disabled the built-in effect renderer board-wide — **including IO Cover and Chipset Accent**, which need the built-in engine *enabled* to render their effect-based static color — and visibly turned off all onboard lighting until a corrective "re-enable everything" command was sent. This is exactly why those two zones were found "stuck off" and not responding to color changes in a later session, well after the CPU-strip work was believed complete: reviving them needed no new protocol, just the `0x32` register cleared back to a state where their bits aren't disabled. `gigabyte_fusion2_static_report` only ever *names* `HDR_D_LED2`'s bit in its disable write — and, deliberately, relies on that same absolute write leaving every other header's bit cleared (enabled), since that's exactly the state IO Cover/Chipset Accent need.

**Two known gaps, accepted rather than solved (see the function's own doc comment for the fuller version):**
- **Not a read-modify-write.** The disable-bitmask write (command `0x32`) sends an absolute byte containing only `HDR_D_LED2`'s bit, not merged with the register's actual current value — there's no read-back capability to merge with. If some other header's disable bit was set by something else (another RGB tool, or a future `colorer` entry for a 4th zone), this call incidentally clears it too. Low-probability for a single-user tool, but real.
- **No rollback on partial-sequence failure.** `set_color_impl` retries the whole 7-report sequence from the start on failure (see "`set_color_impl`'s steps" above), but writes each report to the real device as it goes — if the disable-builtin report succeeds and a later report then fails on every retry, the device can be left worse off than before the call (CPU strip dark with its built-in effect disabled, and/or IO Cover/Chipset Accent stuck on a stale color) until a later successful `set_color` call fixes it.

**Not implemented — deliberately left for a future extension, not fabricated:** dynamic strip-length scanning, per-board calibration reading, `LED_C` (apparently unpopulated on this board), the two unpopulated Gen2 strip headers, and any other Gigabyte board model (a different model would need its own `IMPLEMENTED_PROTOCOLS` entry, its own confirmed zone list, LED counts, and calibration order; none of the constants here should be assumed to transfer).

## Retry and transport abstraction

`retry_with_delay(attempts, delay, op)` (`src/device/hid.rs`) retries a fallible `FnMut` up to `attempts` times, calling `delay()` between attempts but never after the last one, surfacing the final error once exhausted. `delay` is injected specifically so tests never sleep for real. The retried operation re-opens the transport each attempt (`open_transport(&original)` then the `ReportKind`-appropriate write) — this absorbs the brief window where udev hasn't finished applying rules to a just-plugged device.

## Error mapping

`map_hid_error` (`src/device/hid.rs`) distinguishes `hidapi::HidError::IoError` wrapping a `PermissionDenied`-kind `io::Error` — mapped to `DeviceError::PermissionDenied` — from every other `hidapi` error, which becomes `DeviceError::Hid`. This lets command-layer code print udev-setup guidance specifically for the permission case rather than a generic I/O error.

## Known conflict risk: other RGB control software

Nothing else was found holding `/dev/hidraw11` open or running as a service when the Ornata V3 protocol was verified (checked `ps`/`fuser`/`lsof`/`systemctl` — see the session that added this entry). If a userspace RGB daemon that also speaks Razer's protocol (e.g. OpenRGB, `openrazer-daemon`) is running, it can fight over device state with `colorer set` — neither locks the device, and this plan has no concurrent-access coordination (see the sysfs backend's equivalent scope note).
