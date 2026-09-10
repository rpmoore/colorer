# section-02-list-hid

## Goal

`colorer list` enumerates HID devices via `hidapi`, filters to a curated known-RGB-vendor allowlist, and prints a table. This is the first milestone that does something real: it should surface the user's actual keyboard/mouse (or, if their vendor IDs aren't yet in the allowlist, a correct empty-result-with-message — not a bug).

## Dependencies

Depends on **section-01-cli-scaffold**: the `Cli`/`Commands` skeleton (`src/cli.rs`) and `src/main.rs` dispatch loop must already exist and build/run `--help`/`--version` correctly, with `cargo fmt`/`clippy` gates established. This section extends `Commands` with a `List(ListArgs)` variant carrying real args (previously a placeholder) and wires it to an actual command implementation for the first time.

## Crate/directory changes

Add a new dependency to `Cargo.toml`:
- `hidapi` (default Linux `hidraw` backend)

New files to create:
```
src/device/mod.rs      # DeviceInfo, DeviceSource, DeviceCapability, DeviceBackend trait, DeviceError
src/device/hid.rs       # HidBackend: wraps hidapi, unfiltered discovery
src/device/vendors.rs   # known-RGB-vendor-ID allowlist table + lookup
src/commands/mod.rs     # commands module root
src/commands/list.rs    # `list` command: runs backends, filters/merges, formats table
```

Modify:
```
src/cli.rs    # extend Commands with List(ListArgs { all: bool })
src/main.rs   # dispatch Commands::List to run_list
Cargo.toml    # add hidapi
```

**Module boundary rule (load-bearing for testability):** `cli.rs` never touches devices. `device/*.rs` implements a shared trait (`DeviceBackend`) that both the real `HidBackend` and test fakes satisfy. `commands/*.rs` depends on that trait, never on `hidapi` directly. This is what lets `run_list` be tested with zero real hardware.

## Core types (`src/device/mod.rs`)

These are introduced now (M1) even though some variants/fields aren't used until later milestones — they're specified here as the stable contract the rest of the plan builds on. Do not narrow them down to "only what M1 needs"; the full shape below is intentional.

```rust
/// Where a device was discovered.
enum DeviceSource { Hid, Sysfs }

/// What color control the device is known to support, from discovery alone.
/// This is a *discovery-time* guess, not a promise that `set` supports the device —
/// later milestones additionally gate `set` on an actually-implemented protocol
/// for that specific device, not just this capability tag.
enum DeviceCapability { Unknown, SingleColor, MultiColor }

/// One discovered RGB-capable (or possibly-RGB-capable) device.
struct DeviceInfo {
    id: String,                    // stable identifier used by `show <id>` / `set <id>`
    source: DeviceSource,
    label: String,                 // human-readable name (HID product string, or sysfs LED name)
    vendor_id: Option<u16>,        // HID only
    product_id: Option<u16>,       // HID only
    interface_number: Option<i32>, // HID only — which USB interface this entry came from
    usage_page: Option<u16>,       // HID only — populated by the hidraw backend
    usage: Option<u16>,            // HID only — populated by the hidraw backend
    path: String,                  // hidraw device path, or sysfs directory path
    capability: DeviceCapability,
}

/// Errors surfaced by device discovery/control.
enum DeviceError {
    Io(std::io::Error),
    Hid(hidapi::HidError),
    PermissionDenied { path: String },
    NotFound { id: String },
    DeviceGone { id: String },                          // device vanished between lookup and use (unplugged)
    IdentityMismatch { id: String },                    // revalidation before write found a different device at this id
    Unsupported { id: String, operation: &'static str },
}

trait DeviceBackend {
    /// Discover devices this backend knows about. Must not require elevated privileges.
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}
```

**Device identity note:** `DeviceInfo::id` needs a stable, human-typeable scheme (e.g. a short hash or index derived from `source` + `path`). The exact scheme is an implementation detail to settle in this section — pick something deterministic and collision-resistant for genuinely distinct devices (see test below). This id is a *locator*, not a durable physical-device identity; revalidation-before-write is a concern for later `set` milestones, not this one, but don't pick an id scheme that would make that revalidation impossible later (e.g. keep the source path derivable/recoverable from context, since revalidation needs to re-check path/VID/PID/interface).

Not all fields apply to all sources (HID-only fields are `None` for sysfs entries and vice versa) — the type must not force irrelevant fields to be populated. This matters starting now even though sysfs entries don't exist until the next section, because the type is shared.

## Vendor allowlist (`src/device/vendors.rs`)

```rust
/// Returns the vendor name if `vendor_id` is in the known-RGB allowlist.
fn known_vendor(vendor_id: u16) -> Option<&'static str>;
```

A static table of `(vendor_name: &str, vendor_id: u16)` seeded with known RGB peripheral vendors: Corsair, Razer, Logitech, SteelSeries, ASUS. Expand later as real target devices are identified — this seed list is a starting point, not exhaustive.

## HID backend (`src/device/hid.rs`)

`HidBackend` implements `DeviceBackend`, wrapping `hidapi::HidApi::new()` + `device_list()`, mapping each `hidapi::DeviceInfo` to this crate's `DeviceInfo`.

**Firm rule, not just this milestone's choice:** `HidBackend::discover()` always returns every HID device it sees, **unfiltered**. Vendor-allowlist filtering is entirely a `commands/list.rs` concern, never the backend's. This means `--all` never needs a second discovery path, and it keeps the backend trait's contract identical across HID and future sysfs backends (sysfs entries have no vendor ID and are never subject to vendor filtering at all).

## `list` command (`src/commands/list.rs`)

```rust
/// Run the `list` command: discover via all given backends, filter (unless show_all), format as a table.
fn run_list(backends: &[Box<dyn DeviceBackend>], show_all: bool) -> Result<String, DeviceError>;
```

Behavior:
- Takes `&[Box<dyn DeviceBackend>]` — for this section, only `HidBackend` is passed in (the caller in `main.rs` constructs the slice; a future section adds a second backend to the same slice, no signature change needed).
- Applies the known-vendor allowlist filter to HID-sourced entries only (unless `show_all` is true).
- Formats a table with columns: `id`, `source`, `vendor`, `product`, `path`, `vendor_id`, `product_id`, `capability`.
- Prints a clear "no known RGB devices found (try --all)" message when the filtered result is empty — the command always succeeds at *something* even with zero matches, this is not an error case.

## CLI wiring (`src/cli.rs`, `src/main.rs`)

Extend `Commands::List` with real args:

```rust
struct ListArgs { all: bool }  // `--all` flag
```

`src/main.rs` dispatches `Commands::List(args)` to `run_list`, passing `&[Box::new(HidBackend::new(...))]` (or equivalent construction) and `args.all`, then prints the returned table string (or a formatted error on `Err`).

## Tests first

Write these before/alongside implementation, using only `cargo test` (no extra test crates). Device I/O is faked via `DeviceBackend`; nothing here touches real hardware.

**Core types tests (`src/device/mod.rs`):**
- `DeviceError` variants are distinguishable (e.g. via `matches!`) so command code can branch on error kind (`NotFound` vs `DeviceGone` vs `IdentityMismatch` vs `PermissionDenied` vs `Unsupported`).
- A `DeviceInfo` value can be constructed with all HID-only fields (`vendor_id`, `product_id`, `interface_number`, `usage_page`, `usage`) as `None` for a sysfs-sourced entry, and vice versa — the type shouldn't force irrelevant fields to be populated.

**`vendors.rs` tests:**
- `known_vendor(vid)` returns `Some(name)` for each seeded allowlist vendor ID.
- `known_vendor(vid)` returns `None` for an arbitrary non-allowlisted ID.

**`hid.rs` / `list.rs` tests, via a test-only `FakeBackend` implementing `DeviceBackend` by returning a canned `Vec<DeviceInfo>`:**
- A fake HID backend returning a mix of allowlisted and non-allowlisted `DeviceInfo` entries: `run_list` with `show_all: false` includes only the allowlisted ones; `show_all: true` includes all of them.
- `run_list` on an empty (or fully-filtered-out) device list produces the "no known RGB devices found (try --all)" message — not an error and not empty output.
- `run_list`'s table output includes all specified columns (`id`, `source`, `vendor`, `product`, `path`, `vendor_id`, `product_id`, `capability`) for a known fake device.
- Two fake HID devices with the same vendor/product but different `path` produce two distinct `id`s — whatever id scheme is chosen, verify it doesn't collide for genuinely distinct devices.

**CLI parse test additions (extends section-01's `try_parse_from` pattern to the now-real `ListArgs`):**
- `Cli::try_parse_from(["colorer", "list"])` parses successfully into `Commands::List` with default `ListArgs` (`all: false`).
- `Cli::try_parse_from(["colorer", "list", "--all"])` parses with `all: true`.

`FakeBackend` should live in a test module (e.g. `#[cfg(test)]` in `src/commands/list.rs` or a shared test-support module) — it is not part of the production `device/` module tree.

## Manual verification

- `cargo run -- list` runs cleanly on the dev machine — ideally showing the keyboard/mouse, but an empty-with-clear-message result is also an acceptable pass if their vendor IDs aren't in the allowlist yet (that's a real, correct outcome, not a bug).
- `cargo run -- list --all` shows the full unfiltered HID device set for comparison — the practical way to find and add a missing vendor ID to the allowlist.
- `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings` all pass clean.

## Notes / non-goals for this section

- No sysfs backend yet (`section-03-list-sysfs`) — `list` only takes/uses `HidBackend` here, though the `&[Box<dyn DeviceBackend>]` signature is already shaped to accept a second backend without changing.
- No `show` or `set` commands yet.
- No device-write, identity-revalidation, or udev concerns yet — those begin in `section-05-set-hid`. This section is discovery-only and requires no elevated privileges.
- `--json` output, effects/animation, config files are out of scope for the whole plan, not just this section.

## Implementation Notes (actual, post-review)

Implemented as planned: `DeviceBackend`/`DeviceInfo`/`DeviceCapability`/`DeviceSource`/`DeviceError` in `src/device/mod.rs`, unfiltered `HidBackend` in `src/device/hid.rs`, vendor allowlist in `src/device/vendors.rs`, `run_list` + `FakeBackend` in `src/commands/list.rs`, `--all` CLI wiring. All 16 tests pass; all gates clean.

**Deviations from plan:**
- **`hidapi` backend feature.** Plan called for the default Linux hidraw backend; switched to `default-features = false, features = ["linux-native-basic-udev"]` because the build environment lacked `libudev-dev`/pkg-config for the default backend's C build. This backend is pure Rust (sysfs-based, no `libudev.so` linking), arguably a better fit for the spec's "native device access" framing. Documented with a comment in `Cargo.toml`.
- **Id hashing algorithm.** Plan left the id scheme as an implementation detail ("short hash or index"); code review flagged that `std::collections::hash_map::DefaultHasher` (the first-draft implementation) has no cross-Rust-version stability guarantee, which matters since `id` is meant to be a stable locator across separate CLI invocations. Switched to a hand-rolled FNV-1a hash (fixed algorithm, no new dependency). Documented in `docs/knowledge/device/backend-and-id.md`.
- **Error Display + non-zero exit code.** Not explicitly specified by this section's plan text, but required for `main.rs`'s dispatch to be minimally correct (code review caught: `list` was exiting 0 even on discovery failure, and printing raw `Debug` output). Added `impl Display for DeviceError` and `std::process::exit(1)` on error in `main.rs`.
- **`docs/knowledge/device/` added.** Three concept docs, per `AGENTS.md`'s Knowledge Bundle requirement (not called for by this plan file, same pattern as section-01).

**Confirmed empirical finding (the plan's own open question):** real-hardware manual verification showed `hidapi` enumerates one row per HID *interface*, not one per physical device — a single mouse/keyboard can appear as several `list` rows with the same vendor/product id. No dedup policy is implemented; this is documented in `docs/knowledge/device/hid-interface-enumeration.md` and explicitly deferred to section-05, per the plan's own instruction not to guess at this now.