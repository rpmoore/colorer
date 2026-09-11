# colorer — usage guide

CLI to discover and control RGB-capable hardware on Linux via native device access (HID + sysfs
LED-class), no external app dependency.

## Commands

```
cargo run -- list [--all]
```
Lists discovered devices from both backends (HID via `hidapi`, sysfs via `/sys/class/leds`) as a
table. Without `--all`, HID results are filtered to `vendors.rs`'s known-RGB-vendor allowlist;
`--all` shows every HID device. Sysfs results are never allowlist-filtered — classification
(`SingleColor`/`MultiColor`/`VendorColor`) comes from which attribute files are present.

```
cargo run -- show <id>
```
Re-discovers and prints full detail for one device: path, source, capability, HID-only fields
(vendor/product id, interface, usage page/usage) when present, and current color/brightness where
readable. Sysfs reads `multi_intensity`/`color`/`brightness` by capability; HID has no read path
yet and always reports "unknown".

```
cargo run -- set <id> <color>
```
Parses `<color>` (`RRGGBB` or `#RRGGBB` hex) and writes it via whichever backend's `id` prefix
(`hid-`/`sysfs-`) matches.

- **HID**: fully built (revalidation, retry, Output/Feature transport abstraction, multi-report
  sequences). Four real, hardware-verified protocol entries:
  - Razer Ornata V3 (`1532:02a1`, interface 2) — a single Feature report, reverse-engineered from
    `openrazer/openrazer`'s driver source and confirmed live against a physical keyboard.
  - Razer Naga X (`1532:0096`, interface 3) — two Feature reports (scroll wheel zone, side
    underglow zone), sharing the Ornata V3's report-struct builder and framing quirk.
    Reverse-engineered from `openrazer/openrazer` and confirmed live. Needed the real udev rule
    installed (see Permissions below) — unlike the Ornata V3, this device's `hidraw` nodes weren't
    already covered by an unrelated system Razer rule.
  - Razer Tartarus Pro (`1532:0244`, interface 2) — same struct shape as the Ornata V3
    (`openrazer/openrazer` dispatches both through the same `case` block); confirmed live.
  - Gigabyte RGB Fusion 2 (`048d:5711`, interface 1) — a 7-report sequence covering 3 of this
    board's 6 zones: the CPU-area ARGB strip (Gen2 addressable/Direct mode, disable built-in
    effect + 3 chunked LED-color writes), plus the motherboard-logo "IO Cover" and "Chipset
    Accent" LEDs (the simple fixed-color "effect" command), then one shared apply.
    Reverse-engineered from `OpenRGB`'s driver source and confirmed live, all 3 zones changing
    together in one real `set` call. **Scoped to one motherboard model** (Gigabyte X870E AORUS
    PRO) — the board's other 3 zones (`LED_C`, apparently unpopulated; 2 further Gen2 ARGB
    strips, both reporting 0 LEDs on a scan) and case fans (confirmed unresponsive to every zone
    this chip exposes — likely a separate controller) are untouched.

  Every other HID device still returns `Unsupported`. See `docs/knowledge/device/set-color-hid.md`
  for all four protocols' details, an empirically-discovered Razer framing gotcha (hidapi needs an
  explicit report-ID prefix byte even for unnumbered reports), and a Gigabyte behavioral hazard
  (a mis-scoped "disable built-in effect" command briefly turned off all onboard lighting during
  reverse-engineering).
- **Sysfs**: `MultiColor` (writes `multi_intensity` in `multi_index`'s order, scaled against
  `multi_max_intensity`, plus raises `brightness`) and `VendorColor` (writes an `aabbcc` hex
  string to `color`) are implemented and tested. `SingleColor` (fixed-color) devices return
  `Unsupported`. No real `MultiColor`/`VendorColor` sysfs device was available on the dev machine
  to validate the udev rule or run manual hardware verification against — see
  `docs/knowledge/device/set-color-sysfs.md`.

## Permissions

`packaging/udev/71-colorer.rules` grants unprivileged `hidraw` access via `uaccess` tagging, one
line per real device in `IMPLEMENTED_PROTOCOLS` (Ornata V3, Naga X, Tartarus Pro, Gigabyte),
each scoped to the exact interface `colorer` writes to via `ATTRS{bInterfaceNumber}`. Install it
for real
(`sudo cp` into `/etc/udev/rules.d/`, then `sudo udevadm control --reload-rules && sudo udevadm
trigger`) — a live `udevadm trigger` alone or an unplug/replug is not equivalent to the rule
actually being installed, and was not sufficient on the dev machine for the Naga X even after the
rule file existed in the repo but before it was copied into place. The equivalent sysfs-attribute
rule (section-06) is **not yet added** — it needs a
concrete target device's `KERNEL==` name and attribute list, deferred per that section's own
"don't fabricate a target device" instruction. Until a real sysfs RGB device is identified and
that rule is written, sysfs `set` will fail with `PermissionDenied` outside a root shell on any
real `MultiColor`/`VendorColor` device that does show up.

## Where to look

- `docs/knowledge/index.md` — concept-doc bundle root (device backends, CLI, set-color paths).
- `docs/plans/sections/` — per-milestone plan docs, updated in place with "Implementation Status"
  notes on any deviation from the original plan.
- `docs/plans/implementation/code_review/` — per-section review + fix-triage transcripts.

## Next steps (open, not blocking)

1. Reverse-engineer more real RGB-capable HID devices to add further `IMPLEMENTED_PROTOCOLS` rows
   in `src/device/hid.rs` — every device currently on the dev machine (Ornata V3, Naga X, Tartarus
   Pro, Gigabyte) is now covered.
2. `LED_C` and the two other Gen2 ARGB strips on the Gigabyte board appear unpopulated (no
   observed response / 0 LEDs on scan) — nothing further to add there without different hardware.
   A real read-back capability (`HidTransport` is currently write-only) would let strip
   length/calibration be read dynamically instead of hardcoded per board model, useful for a
   different Gigabyte board.
3. Identify a real `MultiColor`/`VendorColor` sysfs LED device to: write the concrete udev rule
   in `packaging/udev/71-colorer.rules`, run manual hardware verification, and consider adding
   write-retry to `SysfsBackend::set_color` (currently has none, unlike the HID path).
4. If other RGB control software (OpenRGB, `openrazer-daemon`) runs concurrently, it can fight
   over device state with `colorer set` — no locking exists on either side.
