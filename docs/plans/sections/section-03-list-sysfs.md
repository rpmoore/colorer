## Section 03: List via sysfs (secondary discovery path)

### Milestone

M2 from `claude-plan.md`. Extends `colorer list` to also scan `/sys/class/leds/`, merging results with the HID list already implemented in `section-02-list-hid`.

### Dependencies

This section depends on **section-02-list-hid** being complete:
- The `DeviceBackend` trait (`src/device/mod.rs`) — `SysfsBackend` implements it.
- `DeviceInfo`, `DeviceSource`, `DeviceCapability`, `DeviceError` types (`src/device/mod.rs`) — reused as-is; `DeviceSource::Sysfs` and `DeviceCapability::SingleColor`/`MultiColor` already exist as variants (declared in the M1 core-types block, used starting here).
- `src/commands/list.rs` and `run_list()` — already accepts `&[Box<dyn DeviceBackend>]`; this section adds a second backend to that slice and extends the merge/formatting logic.
- The `FakeBackend` test seam from section-02 remains usable for HID-side test fixtures; this section adds a parallel, filesystem-based seam (`SysfsBackend::new(root)`) rather than reusing `FakeBackend`, since sysfs classification logic depends on real file presence/contents, not just canned `DeviceInfo` values.

Do not re-derive or restate the `DeviceBackend` trait, `DeviceInfo` struct, or `DeviceError` enum here — import/use them from `src/device/mod.rs` as already defined in section-02.

### Goal

Extend `colorer list` to also scan `/sys/class/leds/`, merging results with the HID list. On this dev machine, sysfs-classified devices may legitimately be zero — that is a correct result, not a bug, per manual verification below.

### Background: sysfs LED class attributes

Devices under `/sys/class/leds/<name>/` expose plain files as attributes. This section classifies each entry by which files are present:
- **Multi-color**: `multi_intensity` and `multi_index` files present (and typically `multi_max_intensity`, `brightness`, `max_brightness`) → `DeviceCapability::MultiColor`.
- **Single-color / fixed-color**: only `brightness`/`max_brightness` present (no `multi_intensity`) → `DeviceCapability::SingleColor`.
- **Vendor color attribute**: some non-standard drivers (e.g. `hid-ite8291r3`-style) expose a `color` file taking a hex-triplet string (`aabbcc`) instead of the standard `multi_*` scheme. This section only needs to *detect and record* this fact at discovery time — it does not write to it. M5 (`section-06-set-sysfs`) is the milestone that actually writes to this attribute, and it depends on this section having already recorded its presence. **This is a firm dependency contract**: M5 must not introduce an unannounced capability that this section never recorded, so whatever representation is chosen here (an added `DeviceCapability` variant, or a separate boolean/field on `DeviceInfo`) must be decided now and must be sufficient for M5 to determine, from discovery output alone, whether a device uses the vendor `color` attribute.

Entries with missing partner attributes (e.g. `multi_index` present but `multi_intensity` absent), unreadable files, or that disappear mid-scan (a device unplugged during the scan) must be **skipped with a logged warning**, not treated as fatal to the whole `discover()` call.

### Files to create/modify

- **Create** `src/device/sysfs.rs`: `SysfsBackend` implementing `DeviceBackend`.
- **Modify** `src/commands/list.rs`: accept both backends, merge results, add `source` column, implement partial-failure policy.
- **Modify** `src/device/mod.rs` only if a new field/variant is needed to record the vendor `color` attribute fact (see above) — keep this change minimal; do not otherwise touch the trait or existing types.

### Implementation details

`src/device/sysfs.rs`:

```rust
impl SysfsBackend {
    /// `root` is the directory to scan, e.g. "/sys/class/leds" in production or a temp dir in tests.
    fn new(root: impl Into<PathBuf>) -> Self;
}

impl DeviceBackend for SysfsBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}
```

- `root` is stored on `SysfsBackend` as a constructor parameter, not hardcoded — this is the key testability seam for this backend (analogous to `FakeBackend` for HID), letting tests point it at a temp directory containing a fake sysfs tree instead of touching the real `/sys` filesystem.
- Production call site (in `commands/list.rs` or `main.rs`, wherever backends are assembled) constructs `SysfsBackend::new("/sys/class/leds")`.
- For each entry under `root`:
  1. Determine capability by probing for `multi_intensity`/`multi_index` (→ `MultiColor`), else `brightness`/`max_brightness` only (→ `SingleColor`), else probe for the vendor `color` attribute and record its presence per the representation decided above.
  2. Populate `DeviceInfo` with `source: DeviceSource::Sysfs`, `label` (the LED class directory name), `path` (the sysfs directory path), `capability` as determined above, and `None` for all HID-only fields (`vendor_id`, `product_id`, `interface_number`, `usage_page`, `usage`).
  3. On missing partner attributes, unreadable files, or a vanished entry: log a warning and skip that entry — do not abort the whole scan.
- `discover()` returns `Ok(vec![])` (not an error) when the root contains zero valid entries — a directory that exists but yields nothing is a normal, successful outcome.
- `discover()` may still return `Err` for a harder failure (e.g. `root` itself unreadable) — this is the case `commands/list.rs`'s partial-failure policy (below) must handle gracefully.

`src/commands/list.rs` updates:

- `run_list` (or its internal implementation) now runs `discover()` across all given backends (HID + sysfs), merging results into one table with a `source` column (`hid` | `sysfs`).
- **Partial-failure policy**: if one backend's `discover()` returns an `Err`, `list` still returns the *other* backend's results plus a visible warning line in the output — it must not fail the whole command just because, say, `/sys/class/leds` isn't readable for some reason. This applies symmetrically (either backend erroring should not sink the other's results), though the HID-error case was already implicitly possible before this section; this section is what actually exercises and tests it, per the sysfs discover-error case below.
- Table formatting: same columns as before (`id`, `source`, `vendor`, `product`, `path`, `vendor_id`, `product_id`, `capability`) — sysfs rows will have empty/`None`-rendered `vendor`, `vendor_id`, `product_id` cells since those are HID-only fields.
- Vendor-allowlist filtering (from section-02) continues to apply only to HID-sourced entries; sysfs entries have no vendor ID and are never subject to vendor filtering.

### Tests (write first)

All tests target `SysfsBackend` (via a temp-directory fixture seam — write real files under a `tempfile`-created directory, no root privileges required, no touching the real `/sys` tree) and `run_list`'s merge/partial-failure logic.

From `claude-plan-tdd.md`, M2 — List via sysfs:

- Test (`sysfs.rs` / temp-dir fixture): a fixture directory with `multi_intensity` + `multi_index` files classifies as `DeviceCapability::MultiColor`.
- Test: a fixture directory with only `brightness`/`max_brightness` classifies as `DeviceCapability::SingleColor`.
- Test: a fixture directory with a vendor `color` file (no `multi_intensity`) is recorded with the vendor-color-attribute fact that M5 will consume (however that's represented — a capability variant or a separate field).
- Test: a fixture entry missing an expected partner attribute (e.g. `multi_index` present but `multi_intensity` absent) is skipped with a warning, not treated as fatal, and doesn't appear in the result set.
- Test: `SysfsBackend::discover()` against a fixture with zero valid entries returns an empty `Vec`, not an error.
- Test (`list.rs` merge): when the sysfs backend's `discover()` returns `Err`, `run_list` still returns the HID backend's results plus a visible warning, rather than propagating the error and failing the whole command.
- Test: merged table correctly labels each row's `source` as `hid` or `sysfs`.

Additional test guidance (not explicit test-list entries but implied by the implementation requirements above — cover these too):
- A fixture with `multi_intensity`/`multi_index` present but files unreadable (e.g. permission-denied simulation, if feasible in a temp dir, or simply absent-but-expected) is skipped, not fatal.
- A fixture root itself that doesn't exist/isn't readable causes `SysfsBackend::discover()` to return `Err`, exercised specifically to drive the `run_list` partial-failure test above.

Use `tempfile` (or equivalent, add as a dev-dependency if not already present from section-02) to build fixture directories with the right files (`multi_intensity`, `multi_index`, `multi_max_intensity`, `brightness`, `max_brightness`, `color`) and contents for each test case.

### Manual verification

`cargo run -- list` now shows a merged table; on this dev machine, confirm whether any sysfs-classified devices actually appear (research suggests this may legitimately be zero, which is a correct result, not a bug). Also confirm: `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings` all pass clean before moving to the next section.

### As-built notes

- **Vendor-color representation**: implemented as a new `DeviceCapability::VendorColor` enum variant (`src/device/mod.rs`), not a separate boolean field — kept the capability check a single match, and the variant carries its own doc comment tying it to the M5/section-06 dependency contract.
- **Classification precedence** (`src/device/sysfs.rs::discover`): `MultiColor` > malformed-multi-skip > `VendorColor` > `SingleColor` > unrecognized-skip. Note this checks the `color` file *before* `brightness`/`max_brightness`, which is the opposite of this doc's background-section prose order ("brightness/max_brightness only → SingleColor, else probe vendor color attribute") — real vendor-color drivers (e.g. `hid-ite8291r3`) typically expose `brightness`/`max_brightness` *alongside* `color`, so checking brightness first would silently misclassify them as `SingleColor` and break the very dependency contract this section commits to. Caught and fixed during code review; pinned by test `vendor_color_wins_over_brightness_when_both_present`.
- **Missing partner attribute**: implemented as `has_multi_intensity != has_multi_index` (XOR) — either one present without the other is treated as malformed and skipped, independent of `brightness`/`color` presence.
- **Unreadable vs. absent files**: not distinguished — classification uses `Path::exists()`, which reports `false` for both a missing file and a stat/permission error. Accepted per this doc's own test-guidance hedge ("or simply absent-but-expected"); noted with an in-code comment rather than special-cased.
- **Partial-failure policy** (`src/commands/list.rs::run_list`): implemented by collecting `Result`s from each backend rather than propagating with `?`; a failing backend contributes a `warning: backend discovery failed: {err}` line (bare `DeviceError` Display text, not the backend's name/type — `DeviceBackend` has no name/label to attribute it to, considered out of scope for this section) and its results are simply absent, while other backends' results still render.
- **Test seam**: `SysfsBackend::new(root)` takes `impl Into<PathBuf>`; tests use `tempfile::tempdir()` fixtures per the plan, added as a `[dev-dependencies]` entry in `Cargo.toml`.
- **Files actually touched**: `src/device/sysfs.rs` (new), `src/device/mod.rs` (capability enum only — trait/other types untouched), `src/commands/list.rs`, `src/main.rs` (wires `SysfsBackend::new("/sys/class/leds")` alongside `HidBackend`), `Cargo.toml`/`Cargo.lock`.
- **Manual verification result**: on this dev machine, sysfs discovery returns real entries (keyboard/numlock/capslock/scrolllock indicator LEDs, ethernet activity LEDs) — all classified `SingleColor`; no `MultiColor`/`VendorColor` devices present. `cargo build`, `cargo test` (27 tests), `cargo fmt --check`, `cargo clippy -- -D warnings` all pass clean.