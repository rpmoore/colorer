# section-04-show-device

## Milestone Mapping

This section implements **M3 — Inspect One Device** from `claude-plan.md` / `claude-plan-tdd.md`.

## Dependencies

This section depends on **section-03-list-sysfs** (which itself depends on section-02-list-hid and section-01-cli-scaffold) being complete. In particular, this section assumes the following already exist and are working, and does not re-specify them:

- `src/cli.rs` — the `Cli`/`Commands` clap definitions, including the existing `Commands::List(ListArgs)` variant.
- `src/device/mod.rs` — the `DeviceInfo`, `DeviceSource`, `DeviceCapability`, `DeviceError` types and the `DeviceBackend` trait (see "Core Types" below for the exact shape needed here).
- `src/device/hid.rs` — `HidBackend` implementing `DeviceBackend`.
- `src/device/sysfs.rs` — `SysfsBackend` implementing `DeviceBackend`, constructed via `SysfsBackend::new(root)`.
- `src/commands/list.rs` — `run_list`, and a test-only `FakeBackend` (implements `DeviceBackend` by returning a canned `Vec<DeviceInfo>`) used in that section's tests. This section reuses the same `FakeBackend` pattern for its own tests (either by referencing the existing test fake if it's accessible from this section's test module, or by adding an equivalent fake local to `show.rs`'s tests — implementer's call, but do not duplicate a second incompatible fake type).

Do not re-implement or modify the HID/sysfs backends in this section; only add a new command on top of the existing `DeviceBackend` trait.

## Goal

`colorer show <id>` prints full detail for a single discovered device, looked up by the `id` produced by prior discovery (e.g. from `colorer list`). This is a read-only, no-privilege-required command — it re-runs discovery, finds the matching device, and formats its full detail, including current color/brightness where that's readable without elevated privileges.

## Core Types Needed (already defined in section-02/section-03, recapped here for reference only — do not redefine)

```rust
/// Where a device was discovered.
enum DeviceSource { Hid, Sysfs }

/// What color control the device is known to support, from discovery alone.
enum DeviceCapability { Unknown, SingleColor, MultiColor }

/// One discovered RGB-capable (or possibly-RGB-capable) device.
struct DeviceInfo {
    id: String,
    source: DeviceSource,
    label: String,
    vendor_id: Option<u16>,
    product_id: Option<u16>,
    interface_number: Option<i32>,
    usage_page: Option<u16>,
    usage: Option<u16>,
    path: String,
    capability: DeviceCapability,
}

/// Errors surfaced by device discovery/control.
enum DeviceError {
    Io(std::io::Error),
    Hid(hidapi::HidError),
    PermissionDenied { path: String },
    NotFound { id: String },
    DeviceGone { id: String },
    IdentityMismatch { id: String },
    Unsupported { id: String, operation: &'static str },
}

trait DeviceBackend {
    /// Discover devices this backend knows about. Must not require elevated privileges.
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}
```

`DeviceError::NotFound { id }` already exists as a variant on the shared error type; this section is the first one to actually construct and return it (M3 is the "id not found" use case referenced in the type's doc comment).

## What Changes

### 1. `src/commands/show.rs` (new file)

Add the `show` command implementation:

```rust
/// Run the `show` command: discover across all given backends, find the device
/// matching `id`, and format its full detail. Returns `DeviceError::NotFound`
/// if no discovered device has this id.
fn run_show(backends: &[Box<dyn DeviceBackend>], id: &str) -> Result<String, DeviceError>;
```

Behavior:

- Re-run discovery across **both** backends passed in (in production, `HidBackend` and `SysfsBackend`; in tests, `FakeBackend` instances) — `show` does not maintain any cached state from a prior `list` invocation; it discovers fresh every time it runs, exactly like `list` does.
- Search the combined discovered set for a `DeviceInfo` whose `id` matches the argument. If none matches, return `DeviceError::NotFound { id: id.to_string() }`.
- On a match, format and return the full detail as a `String`, including:
  - `path`, `source`, `capability`
  - vendor/product IDs and interface/usage fields where applicable (i.e. when `Some` — these are HID-only fields and will be `None` for sysfs-sourced entries; don't print misleading zero/empty values for fields that are legitimately absent)
  - current color/brightness, where readable without elevated privileges:
    - For sysfs-sourced devices, `brightness`/`multi_intensity` are plain-readable files — read and include them if present and readable.
    - For HID-sourced devices, there is no read path implemented yet (that's a post-M4/M5 concern) — print `"unknown"` for current color/brightness rather than attempting a device read or blocking on this.
- Unlike `list`'s partial-failure policy (section-03), `show`'s handling of a backend `discover()` error is not separately specified by the plan beyond "returns `DeviceError::NotFound` if the id doesn't match any discovered device" — a reasonable, consistent approach is to apply the same partial-failure tolerance as `list` (skip a failing backend, search what the other backend(s) return, and still produce `NotFound` rather than propagating the backend error, if no match is found in the surviving results). Keep this consistent with how `list` already handles a partial backend failure, since `show` reuses the same discovery step.
- Must not panic on any input, including an empty or malformed `id` string — always returns a `Result`.

### 2. `src/cli.rs` (modify)

Add a new subcommand variant:

```rust
/// Available subcommands.
enum Commands {
    List(ListArgs),
    Show(ShowArgs),
}

struct ShowArgs {
    /// The device id, as printed by `colorer list`.
    id: String,
}
```

Wire `Commands::Show` into whatever dispatch exists in `src/main.rs` for `Commands::List`, calling `run_show` and printing its `Ok(String)` result, or formatting `DeviceError` as a clean CLI error (not a raw `Debug`-printed error, not a panic) on `Err`.

## Tests (write first, per TDD)

All tests live alongside `show.rs` (or in a `#[cfg(test)] mod tests` within it), using the same `FakeBackend`-style seam as section-02/section-03's tests — no real hardware or filesystem access.

- Test (`FakeBackend`): `run_show` with a matching `id` returns formatted detail including all `DeviceInfo` fields (vendor/product IDs, interface/usage where `Some`, path, source, capability).
- Test: `run_show` with a non-matching `id` returns `DeviceError::NotFound`, not a panic. Verify via `matches!(result, Err(DeviceError::NotFound { .. }))` (or equivalent), consistent with the "Core Types" test requirement that `DeviceError` variants are distinguishable via `matches!`.
- Test: for a fake sysfs device with a readable `brightness`/`multi_intensity` fixture, `run_show`'s output includes the current color/brightness.
- Test: for a fake HID device (no read path exists yet, pre-M4), `run_show`'s output shows `"unknown"` for current color/brightness rather than erroring.
- Test: `Cli::try_parse_from(["colorer", "show", "<some-id>"])` parses successfully into `Commands::Show(ShowArgs { id: "<some-id>".into() })` (extends this repo's standing `try_parse_from` test requirement — established in section-01 — to the newly-added `Show` variant/args).
- Test: `Cli::try_parse_from(["colorer", "show"])` (missing required `id` positional argument) produces a clear parse error, not a panic.
- Standing check: `Cli::command().debug_assert()` (already exists from section-01) continues to pass after adding the `Show` variant — no new test needed, just don't break the existing one.

## Manual Verification

- `cargo run -- show <id>` (using an id printed by `colorer list`) shows correct detail matching what `list` reported for that device.
- `cargo run -- show <invalid-id>` produces a clean, readable CLI error (e.g. "device not found: <id>"), not a crash or raw `Debug` dump.
- `cargo build`, `cargo test`, `cargo fmt --check`, and `cargo clippy -- -D warnings` all pass clean, per this project's standing verification gates (established in section-01, restated in `claude-plan.md`'s Verification Summary).

## As-built notes

- **Partial-failure handling**: unlike `list` (which always returns `Ok` and can embed a `warning:` line in its output string), `show`'s `Err(NotFound)` path has no string body to carry a warning in. Implemented as `eprintln!("warning: backend discovery failed: {err}")` per failing backend at discovery time, rather than embedding it in the returned string — kept the `Result<String, DeviceError>` contract unchanged.
- **Capability-to-string**: extracted `impl fmt::Display for DeviceCapability` into `src/device/mod.rs` (byte-for-byte identical strings to `list.rs`'s prior inline match), reused by both `list`'s table and `show`'s detail output, rather than duplicating the match in a third place.
- **Sysfs current-state attribute selection**: `multi_intensity` for `MultiColor`, `color` for `VendorColor`, `brightness` for `SingleColor`/`Unknown` — read via `Path::new(&d.path).join(attr)`, trimmed, falling back to `"unknown"` on missing/unreadable/empty content.
- **Hex formatting**: `vendor_id`/`product_id` print as plain lowercase hex (matches `list`'s existing table convention); `usage_page`/`usage` print as `{:#06x}` (zero-padded with `0x` prefix) — these are new show-only fields with no prior convention, so no attempt was made to unify the two styles.
- **Duplicate-id assumption**: `.find()` takes the first matching device; documented via comment rather than special-cased, since `make_id` prefixes by source and production registers only one instance per backend type.
- **Files actually touched**: `src/commands/show.rs` (new), `src/cli.rs` (`Commands::Show`/`ShowArgs`, plus non-exhaustive-match fixes to two pre-existing tests), `src/commands/mod.rs`, `src/device/mod.rs` (`Display` for `DeviceCapability`), `src/commands/list.rs` (switched to the shared `Display` impl instead of its own inline match), `src/main.rs` (`Commands::Show` dispatch, shared `backends()` helper).
- **Manual verification result**: `cargo run -- show <id>` (using an id from `cargo run -- list`) matched `list`'s reported detail exactly; `cargo run -- show <bad-id>` printed `error: device not found: <bad-id>` and exited 1, no panic/raw Debug dump. `cargo build`, `cargo test` (37 tests), `cargo fmt --check`, `cargo clippy -- -D warnings` all pass clean.

- No color/brightness *writes* — this is a read-only command. Setting color is section-05 (HID) and section-06 (sysfs).
- No changes to `HidBackend` or `SysfsBackend`'s discovery/classification logic — those are section-02/section-03 concerns and are assumed complete and correct going into this section.
- No `--json` or other new output formats — plain formatted text only, consistent with the rest of the plan's explicitly-out-of-scope list.