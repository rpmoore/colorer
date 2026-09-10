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
- [HID set-color path](set-color-hid.md) — `set_color_impl`'s revalidation/retry machinery; `IMPLEMENTED_PROTOCOLS` is empty, so `set` currently returns `Unsupported` for every real device
