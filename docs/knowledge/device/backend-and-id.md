---
type: concept
title: Device Backend Trait and Id Scheme
description: DeviceBackend is the shared discovery trait; DeviceInfo::id is derived via a stable FNV-1a hash, not stdlib DefaultHasher.
resource: colorer/device
tags: [device, testability]
---

# Device Backend Trait and Id Scheme

`src/device/mod.rs:66-69` defines `DeviceBackend`, the trait every discovery source (currently `HidBackend`, `src/device/hid.rs`) implements. `discover()` must not require elevated privileges — enumeration always works unprivileged; only later `set` operations need permission handling.

`src/commands/list.rs:48-56` is the only place vendor-allowlist filtering happens. Backends always return everything they see, unfiltered (`src/device/hid.rs` doc comment) — this keeps `--all` from needing a second discovery path and keeps the trait's contract identical across backends (sysfs entries, added in section-03, have no vendor id and are never filtered).

## Id scheme

`DeviceInfo::id` (`src/device/mod.rs`, `make_id`) is `"{source}-{hash:08x}"` where `hash` is a 32-bit **FNV-1a** hash of the device's `path`, hand-rolled in `fnv1a()` — deliberately *not* `std::collections::hash_map::DefaultHasher`. The stdlib explicitly does not guarantee `DefaultHasher`'s algorithm is stable across Rust releases, and `id` is meant to be a stable locator a user types into `show <id>`/`set <id>` (introduced in later sections) across separate invocations, potentially built with different compiler versions over time. FNV-1a's algorithm is fixed and documented, so the same `path` always produces the same `id` regardless of toolchain.

**Invariant to preserve:** don't swap `make_id`'s hash back to a stdlib-provided general-purpose hasher (`DefaultHasher`, `RandomState`-backed hashers) — both carry no cross-version/cross-process stability guarantee, which this id scheme specifically needs.
