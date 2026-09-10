# section-06-set-sysfs

## Implementation Status (as built)

`SysfsBackend::set_color` and the pure `scale_multi_intensity` helper are implemented in
`src/device/sysfs.rs` and wired into `color_writers()` in `src/main.rs`, per the "What Changes
§1" section below. All 6 "Tests First" scenarios are covered by fixture/unit tests in
`src/device/sysfs.rs`'s test module.

**Deferred** (per the milestone's own conditional clause — see below): this dev machine's
`/sys/class/leds` has no real `MultiColor`/`VendorColor` device, only `SingleColor` fixed-color
LEDs (keyboard caps/num/scroll-lock, network link LEDs). Per this section's explicit instruction
not to fabricate a target device, the following are **not done** and remain open until a real
sysfs RGB target is identified:

- `packaging/udev/71-colorer.rules` sysfs-attribute addition ("What Changes §2") — needs a
  concrete `KERNEL==` name and attribute file list for the real target device; no placeholder
  was shipped.
- `packaging/udev/README.md` update for the sysfs rule.
- Manual hardware verification (`cargo run -- set <id> ff0000` against real hardware).

Code review (`docs/plans/implementation/code_review/section-06-review.md`) also flagged, and this
implementation fixed: a u32 overflow in the intensity-scaling multiply (widened to u64), and
missing validation that `multi_index` names exactly red/green/blue once each (previously a
malformed channel list would silently zip wrong/duplicate channels instead of erroring).

## Milestone

M5 — Set Color, sysfs Path (final milestone; **conditional** on M2/`section-03-list-sysfs` having actually found a real sysfs-exposed RGB target on the dev machine — if no sysfs-classified device exists, this milestone may be skipped or deferred until one does. Do not fabricate a target device to force this milestone through if the real machine has none; only proceed once a concrete sysfs LED class device with a known attribute layout is confirmed present).

## Goal

`colorer set <id> <color>` also works for a sysfs-exposed device (`/sys/class/leds/...`), completing the `set` command's coverage of both backends established earlier (HID in `section-05-set-hid`, sysfs discovery in `section-03-list-sysfs`).

## Dependencies (reference only — do not re-implement here)

- **`section-03-list-sysfs`**: provides `SysfsBackend` (in `src/device/sysfs.rs`) with `SysfsBackend::new(root: impl Into<PathBuf>)`, the `DeviceCapability::{Unknown, SingleColor, MultiColor}` classification already performed during discovery, and — critically — the vendor `color` hex-attribute probing recorded at discovery time (the `hid-ite8291r3`-style interface). This section must not invent a new discovery-time fact that M2/section-03 never recorded; if the vendor-`color`-attribute presence isn't already captured on `DeviceInfo` (or wherever M2 stashed it), that capture belongs in section-03, not here — this section only *consumes* it.
- **`section-05-set-hid`**: provides `src/color.rs` (`Rgb { r: u8, g: u8, b: u8 }`, `parse_color`), the `DeviceError` enum (all variants, including `Unsupported`, `NotFound`, `DeviceGone`, `IdentityMismatch`, `PermissionDenied`), the `set_color`-style trait/method pattern already established on `HidBackend`, `src/commands/set.rs` and the `Commands::Set(SetArgs { id: String, color: String })` CLI wiring, and the existing `packaging/udev/71-colorer.rules` file (this section adds to that file, not creates it).
- The `DeviceBackend` trait and core types (`DeviceInfo`, `DeviceSource`) come from `section-02-list-hid`.

This section assumes all of the above already exist and compile. It only adds sysfs-specific `set_color` behavior and a sysfs-specific udev rule addition.

## Background

Linux `/sys/class/leds/<name>/` devices expose color control through plain files:

- `brightness` (writable) / `max_brightness` (read-only ceiling) — always present.
- For multi-color LEDs: `multi_intensity` (writable, all channel values in **one** space-separated write — partial writes are invalid per kernel docs) and `multi_index` (read-only, names the channel order, e.g. `"green blue red"` means the first value written to `multi_intensity` is green, not red) and `multi_max_intensity` (read-only per-channel ceiling, commonly but not always 255).
- The effective emitted brightness is `led_brightness = brightness * multi_intensity / max_brightness` — this means an all-zero `brightness` makes any `multi_intensity` write invisible even though the write itself succeeds. `set_color` must ensure `brightness` is set to a usable (typically max) level as part of setting a color, not just write `multi_intensity` in isolation.
- Some vendor drivers (e.g. `hid-ite8291r3`-class) expose a non-standard `color` file taking a raw `aabbcc`-style hex triplet instead of the standard `multi_intensity`/`multi_index` pair. M2/section-03 already probed for and recorded this.
- `SingleColor` devices (only `brightness`/`max_brightness`, no multi-color or vendor-color attribute) are fixed-hardware-color LEDs — they cannot change emitted color, only brightness, and this plan does not add a brightness-only command, so `set_color` must return `DeviceError::Unsupported` for these rather than attempting anything.

## What Changes

### 1. `src/device/sysfs.rs` (modify)

Implement `set_color` for `SysfsBackend`, gated on the `DeviceCapability` (and vendor-color-attribute fact) already recorded at discovery time:

- **Revalidation first** (same principle as M4/section-05's HID path): before writing, confirm the device directory at the `id`'s path still exists and still matches what was discovered (still the same LED class directory, attributes still present) — return `DeviceError::DeviceGone` or `DeviceError::IdentityMismatch` as appropriate rather than blindly writing to whatever now occupies that path.
- **`MultiColor` devices**:
  1. Read `multi_index` at write time (do not hardcode an assumed order — e.g. `"green blue red"` means the write order is G, B, R).
  2. Read `multi_max_intensity` at write time per channel (do not assume 255; scale the incoming 8-bit `Rgb` channel value linearly against the actual ceiling).
  3. Write all channel values in a single `multi_intensity` write, in the order dictated by `multi_index`, computed from the (possibly non-255) per-channel ceilings.
  4. Also write `brightness` to a usable (typically max, i.e. `max_brightness`) level as part of the same color-set operation — a `multi_intensity` write alone is insufficient since `led_brightness` is a product of both.
- **Vendor `color`-attribute devices** (probed for by M2/section-03, no `multi_intensity`): write the `aabbcc`-format hex string directly to the `color` file, derived from the `Rgb` value.
- **`SingleColor` (fixed-color) devices**: return `DeviceError::Unsupported` for any color-change attempt — do not write anything.

Suggested shape (fill in per existing `SysfsBackend`/trait conventions established in section-05):

```rust
impl SysfsBackend {
    /// Attempt to set `color` on the sysfs LED device identified by `id`.
    /// Revalidates identity before writing. Returns:
    /// - `DeviceError::Unsupported` for SingleColor (fixed-color) targets.
    /// - `DeviceError::DeviceGone` / `IdentityMismatch` if the device changed/vanished.
    /// - Ok(()) after successfully writing brightness + multi_intensity (MultiColor)
    ///   or the color file (vendor-color-attribute devices).
    fn set_color(&self, id: &str, color: Rgb) -> Result<(), DeviceError> { todo!() }
}
```

Internal helper(s) worth isolating for testability (mirrors M4/section-05's "pure function builds the outgoing bytes separately from the open-device call" pattern):

```rust
/// Given a channel order (from `multi_index`) and per-channel max intensities
/// (from `multi_max_intensity`), compute the values to write to `multi_intensity`
/// for a given `Rgb`, in the correct write order. Pure — no I/O.
fn scale_multi_intensity(
    order: &[MultiChannel],       // parsed "red"/"green"/"blue" tokens from multi_index
    max_intensity: &[u32],        // parallel per-channel ceiling, one per `order` entry
    color: Rgb,
) -> Vec<u32> { todo!() }
```

(`MultiChannel` or equivalent — however the channel-name tokens from `multi_index` are represented — is an implementation detail; keep it internal to `sysfs.rs` unless section-03 already introduced such a type.)

### 2. `packaging/udev/71-colorer.rules` (modify, append)

This file already exists from `section-05-set-hid` with a `hidraw`-scoped `uaccess` rule. Add a **separate** rule mechanism for the sysfs attribute path(s) — `uaccess` tagging does not apply the same way to plain sysfs attribute files, so this needs a `RUN+=`-based `chgrp`/`chmod` rule instead.

This addition must specify, concretely (not left for the implementer to infer at review time):
- The exact group name to `chgrp` the attribute file(s) to.
- The exact resulting file mode (`chmod` target) — not a recursive or world-writable grant.
- The exact attribute file list involved for the specific target device gated by this milestone (e.g. `multi_intensity` and `brightness`, or just the vendor `color` file — whichever applies to the actual concrete device this milestone targets).

Example shape (fill in the real group/mode/path list for the actual target device once identified — do not ship a placeholder VID/PID/path):

```
# sysfs LED attribute access for colorer (M5) — scoped to the specific target device path
SUBSYSTEM=="leds", KERNEL=="<target-led-name>", RUN+="/bin/chgrp <group> /sys/class/leds/%k/multi_intensity /sys/class/leds/%k/brightness", RUN+="/bin/chmod 0664 /sys/class/leds/%k/multi_intensity /sys/class/leds/%k/brightness"
```

Update `packaging/udev/README.md` (added in section-05) with the sysfs-specific install note if the reload/trigger steps differ at all for this rule type (they generally don't — same `udevadm control --reload-rules && udevadm trigger`), and note that manual verification must check actual effective file permissions/group ownership on the running system, not just that the rule file is present.

## Tests First

All sysfs `set_color` tests use the same temp-directory fixture seam as M2/`section-03-list-sysfs`'s `SysfsBackend::new(root)` tests — never touch the real `/sys` tree.

1. **Channel-order test**: for a `MultiColor` fixture with `multi_index = "green blue red"`, `set_color` writes `multi_intensity` in G, B, R order for a given `Rgb` — explicitly not assumed R, G, B order. Assert on the exact fixture-file contents written.

2. **Brightness-accompanies-color test**: `set_color` also ensures `brightness` is set to a usable level as part of a color-set — verify the fixture's `brightness` file receives an appropriate write (not just that `multi_intensity` was written), given a non-max starting `brightness`.

3. **Non-255 ceiling scaling test**: 8-bit `Rgb` channel values scale correctly against a fixture's `multi_max_intensity` when it is **not** 255 — verify the actual scaling math (e.g. against a ceiling like 15 or 100), not just the trivial 255-ceiling identity case. This should be covered by a focused unit test on the pure `scale_multi_intensity`-style helper in addition to (or instead of) a full fixture-based `set_color` test.

4. **Vendor `color`-attribute test**: for a fixture with the vendor `color` attribute (as probed for in M2/section-03), `set_color` writes the correct `aabbcc`-format hex string to that file.

5. **Fixed-color `Unsupported` test**: for a `SingleColor` (fixed-color) fixture, `set_color` returns `DeviceError::Unsupported` and does not write to any file (assert no mutation of the fixture's `brightness`/other files occurred).

6. **Revalidation tests** (mirroring M4/section-05's pattern, applied to sysfs paths): a fixture where the device directory at `id` no longer exists returns `DeviceError::DeviceGone`; one where it exists but no longer matches the originally-discovered identity (e.g. attributes changed shape) returns `DeviceError::IdentityMismatch`. Include these if not already fully covered by a shared revalidation helper from section-05 — if `SysfsBackend` reuses common revalidation logic already tested there, a thin sysfs-specific test confirming it's wired in is sufficient rather than re-testing the whole matrix.

Write all of these tests before implementing the corresponding `set_color` logic in `src/device/sysfs.rs`.

## Manual Verification

After installing the sysfs-specific udev rule addition (reload rules, trigger, confirm actual effective file permissions for the logged-in session — not just that the rule file exists), `cargo run -- set <id> ff0000` changes the real sysfs-exposed device's color and it is visibly correct on the hardware (not just "the write call returned Ok") — this confirms the brightness/intensity interaction (`led_brightness = brightness * multi_intensity / max_brightness`) was actually handled correctly, not just that a write occurred.

## Verification Gates (apply here as everywhere in this plan)

`cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings` must all pass before this milestone is considered done, in addition to the manual hardware verification above. `cargo test` must not require real hardware or `/sys` access — all sysfs `set_color` logic is exercised through the `SysfsBackend::new(root)` temp-directory seam.

## Scope Notes

- Multi-process/concurrent device access coordination is out of scope here, same as the rest of this plan (no locking).
- Exhaustive partial-write/protocol-level failure recovery beyond clean identity revalidation and the writes specified above is out of scope — this section defines only what kernel `sysfs` LED-class semantics make genuinely knowable (single-shot `multi_intensity` writes, brightness-multiplier interaction, per-channel scaling).
- This is the last section in the plan; there is no further section depending on this one.