---
type: concept
title: HID Interface Enumeration Produces Multiple Rows Per Device
description: hidapi enumerates one DeviceInfo per HID interface, so one physical device can appear as several list rows; no dedup policy exists yet.
resource: colorer/device
tags: [device, hid, open-question]
---

# HID Interface Enumeration Produces Multiple Rows Per Device

`HidBackend::discover()` (`src/device/hid.rs`) maps each entry from `hidapi::HidApi::device_list()` to one `DeviceInfo`. `hidapi` enumerates one entry per HID **interface**, not one per physical device — confirmed empirically against real hardware during section-02: a single mouse/keyboard with multiple interfaces (e.g. separate report descriptors for standard input vs. vendor-specific control) produces multiple `DeviceInfo` rows sharing the same `vendor_id`/`product_id` but different `path`/`interface_number`.

This was anticipated by the original plan (`docs/plans/claude-plan.md`'s "Device identity" section, and `docs/plans/sections/section-02-list-hid.md`'s "Device identity note") as an open question to resolve empirically rather than guess at upfront — it's confirmed now, but **no deduplication policy has been decided or implemented yet**. `colorer list` today shows every interface as a separate row.

**Open question, not yet resolved:** whether multiple HID interfaces of one physical device should collapse into a single `DeviceInfo` (and if so, by what key — same vendor/product/serial?) or remain separate entries (since a `set` implementation for a specific device may need to target one particular interface, not the "whole device"). Resolve this when section-05 (`set`, HID path) needs to pick a specific interface to write to — that milestone's precondition (identify one concrete device + working protocol) is the natural point to also decide this, since the protocol work will reveal which interface actually needs writes.
