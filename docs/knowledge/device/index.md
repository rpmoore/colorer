---
type: index
title: Device
description: Device discovery backends, identity scheme, and shared error type for colorer.
resource: colorer/device
tags: [device]
---

# Device

- [Backend trait and id scheme](backend-and-id.md) — `DeviceBackend`, `DeviceInfo`, `make_id`
- [HID interface enumeration](hid-interface-enumeration.md) — why one physical device can produce multiple discovered rows
- [Sysfs LED classification](sysfs-classification.md) — `SysfsBackend`'s attribute-file classification precedence, `DeviceCapability::VendorColor`, and `list`'s partial-failure merge policy
- [HID set-color path](set-color-hid.md) — `set_color_impl`'s revalidation/retry machinery; four hardware-verified `IMPLEMENTED_PROTOCOLS` entries: Razer Ornata V3, Naga X, and Tartarus Pro (shared struct builder, Feature report, empirically-required report-ID prefix byte) and Gigabyte RGB Fusion 2's CPU ARGB strip (Gen2 addressable-strip protocol, one header only)
- [Sysfs set-color path](set-color-sysfs.md) — `set_color_core`'s revalidation/dispatch for `MultiColor`/`VendorColor` devices; udev rule and hardware verification deferred pending a real target device
