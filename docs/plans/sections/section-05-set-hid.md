## Section 5: Set Color, HID Path (M4)

### Dependencies

This section builds directly on **section-02-list-hid** (must be complete and merged first):
- The `DeviceInfo` struct, `DeviceSource`, `DeviceCapability`, `DeviceError`, and `DeviceBackend` trait defined in `src/device/mod.rs`.
- The `HidBackend` in `src/device/hid.rs` (wraps `hidapi::HidApi`, unfiltered `discover()`).
- The `FakeBackend` test seam used for backend-level fakes.

This section does **not** depend on section-03 (sysfs) or section-04 (show) — it only needs HID discovery to exist. It **blocks** section-06-set-sysfs, which reuses `color.rs`, the `HidTransport`-style testing pattern, and the `packaging/udev/` directory this section creates.

Reference only — do not re-implement anything from section-02 here. If any type signature below differs from what section-02 actually produced, use the real signatures from that section's code, not this section's paraphrase.

### Goal

`colorer set <id> <color>` sets the color of one concrete, user-identified HID peripheral. This is the first milestone requiring device write access and the first requiring real per-device protocol work.

**Precondition:** before implementation starts, the user identifies one specific real device (exact make/model) to target, AND a usable protocol is established for it (the correct HID interface/usage, and a known or reverse-engineered report format). Identifying the device model alone is not sufficient — model identification doesn't guarantee a usable protocol is actually known yet. If this precondition isn't met, stop and get it resolved before writing `set_color` logic (the surrounding scaffolding — `color.rs`, `HidTransport`, retry logic, udev rule structure — can still be built and tested against a fake/placeholder device in the interim, since none of that requires the real protocol to be known).

### Background context

`colorer` is a Linux-only Rust CLI discovering and controlling RGB hardware via native device access (USB HID + sysfs), no OpenRGB dependency. Setting colors requires elevated device permissions by default; the fix is a packaged `udev` rule installed once, not `sudo` per invocation. Discovery (`list`/`show`) never requires elevated privileges — that invariant must not be broken by this section (only `set` needs permission handling).

Existing core types from section-02 (`src/device/mod.rs`), reproduced here for reference only:

```rust
enum DeviceSource { Hid, Sysfs }
enum DeviceCapability { Unknown, SingleColor, MultiColor }

struct DeviceInfo {
    id: String,
    source: DeviceSource,
    label: String,
    vendor_id: Option<u16>,        // HID only
    product_id: Option<u16>,       // HID only
    interface_number: Option<i32>, // HID only
    usage_page: Option<u16>,       // HID only
    usage: Option<u16>,            // HID only
    path: String,                  // hidraw device path, or sysfs directory path
    capability: DeviceCapability,
}

trait DeviceBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}
```

`DeviceError` (from section-02, this section adds new variants to it — see below):

```rust
enum DeviceError {
    Io(std::io::Error),
    Hid(hidapi::HidError),
    PermissionDenied { path: String },
    NotFound { id: String },
    DeviceGone { id: String },        // device vanished between lookup and use (unplugged)
    IdentityMismatch { id: String },  // revalidation before write found a different device at this id
    Unsupported { id: String, operation: &'static str },
}
```

If section-02 did not already add `DeviceGone`, `IdentityMismatch`, and `Unsupported` to `DeviceError`, add them as part of this section — they are required for the work below.

**Device identity note:** `DeviceInfo::id` is a locator (derived from `source` + `path`), not a durable physical-device identity. Before any write, the targeted backend must revalidate that the device at `id` still exists and still matches the identity fields it was discovered with (VID/PID/interface for HID), returning `DeviceError::DeviceGone` or `DeviceError::IdentityMismatch` rather than silently writing to whatever now occupies that path.

### Tests first

Write these before implementation (extracted from `claude-plan-tdd.md`, M4 section). All tests run via `cargo test`, no real hardware or elevated privileges required — device I/O is faked via `HidTransport` and backend-level fakes.

1. **`color.rs` — `parse_color`:**
   - Accepts `"#RRGGBB"` and `"RRGGBB"` forms and produces the correct `Rgb`.
   - Rejects malformed input: wrong length, non-hex characters, missing/extra `#`.

2. **Report-building pure function:** given a known `Rgb`, the byte-level report construction produces the expected byte sequence for the targeted device's protocol. (Exact expected bytes depend on the concrete device identified per the precondition above — fill in once the real protocol is known.)

3. **Fake `HidTransport` — happy path:** `set_color` calls `write_report` with the expected bytes for a valid target device.

4. **Unsupported device:** `set_color` against an `id` whose device isn't the one implemented protocol-wise returns `DeviceError::Unsupported`.

5. **Not found:** `set_color` against a nonexistent `id` returns `DeviceError::NotFound`.

6. **Identity revalidation:**
   - A fake backend where the device at `id` no longer matches its originally-discovered VID/PID/interface returns `DeviceError::IdentityMismatch`.
   - A fake backend where the device at `id` is gone entirely returns `DeviceError::DeviceGone`.

7. **Bounded retry:**
   - A fake `HidTransport` that fails open/write a bounded number of times then succeeds is retried up to the fixed attempt count and eventually succeeds.
   - A fake `HidTransport` that always fails exhausts the bounded retry and surfaces the underlying error.
   - Verify the retry count is actually bounded (does not retry forever) and the injectable delay function is called, not a real `sleep` (tests must not actually sleep).

8. **Permission-denied surfacing:** a fake `HidTransport` simulating a permission-denied error surfaces `DeviceError::PermissionDenied` distinctly (not conflated with other I/O errors), so command-layer code can print "see udev setup" guidance specifically for this case.

Also, from the Core Types section of the TDD companion (applies here since this section extends `DeviceError`):
- `DeviceError` variants must be distinguishable via `matches!` so command code can branch on error kind (`NotFound` vs `DeviceGone` vs `IdentityMismatch` vs `PermissionDenied` vs `Unsupported`).

### Implementation details

**New/changed files:**

```
src/color.rs                    # NEW: Rgb type + hex color parsing
src/device/mod.rs               # MODIFIED: add DeviceGone/IdentityMismatch/Unsupported to DeviceError if not already present; add set_color capability
src/device/hid.rs               # MODIFIED: add HidTransport trait, set_color impl for the one targeted device
src/cli.rs                      # MODIFIED: add Commands::Set(SetArgs)
src/commands/set.rs              # NEW: `set` command implementation
packaging/udev/71-colorer.rules  # NEW: scoped udev rule
packaging/udev/README.md         # NEW (or a README section): install steps
```

**1. `src/color.rs`:**

```rust
/// An RGB color value.
struct Rgb { r: u8, g: u8, b: u8 }

/// Parses a color string, e.g. "#RRGGBB" or "RRGGBB". Returns an error for anything else.
fn parse_color(input: &str) -> Result<Rgb, ColorParseError>;
```

Define a `ColorParseError` type (implementer's call on exact variants — e.g. wrong length, invalid hex digit) sufficient to give a clear CLI error message.

**2. Injectable HID transport seam** (in `src/device/hid.rs` or a new small module — implementer's call, keep it near `hid.rs`):

```rust
/// Abstracts the raw HID write so it can be faked in tests.
trait HidTransport {
    fn write_report(&self, report: &[u8]) -> Result<(), DeviceError>;
}
```

The production implementation wraps an opened `hidapi::HidDevice`. A test fake records calls and can simulate failures (short write, timeout-style error, permission-denied) without a real device.

**3. `set_color` capability.** Extend `DeviceBackend` (or add a separate, more narrowly-implemented trait — implementer's call, but keep it small) with:

```rust
/// Attempt to set `color` on the device identified by `id`. Revalidates the
/// device's identity (path still exists, VID/PID/interface still match) before
/// writing. May fail with DeviceError::Unsupported if this specific device's
/// protocol isn't implemented, DeviceError::PermissionDenied if the udev rule
/// isn't installed, or DeviceError::DeviceGone/IdentityMismatch if the device
/// changed or disappeared since discovery.
fn set_color(&self, id: &str, color: Rgb) -> Result<(), DeviceError>;
```

Implement this only for the specific targeted device in `HidBackend`, returning `DeviceError::Unsupported` for any device whose protocol isn't actually implemented.

**Firm rule:** `set_color` must check against an explicit list of devices this code actually implements a protocol for, not just "was it in the vendor allowlist" from `list`. Being allowlist-eligible for `list` does not imply `set` support — a device can be discoverable and shown in `list --all`/`list` while still being `Unsupported` for `set`.

**Report-building isolation:** the specific device protocol logic (byte-level HID report construction) must be isolated into a pure function that builds the outgoing report bytes from an `Rgb`, separately testable without an actual open device handle. This is what test #2 above targets directly.

**Revalidation-before-write:** before calling into `HidTransport::write_report`, re-check that the device at `id` still exists and its VID/PID/interface still match what was recorded at discovery time. Return `DeviceError::DeviceGone` if it's gone, `DeviceError::IdentityMismatch` if something else now occupies that id/path.

**4. Bounded retry for the permission race.** Opening a just-plugged device can transiently fail before udev finishes applying rules. Handle this with a bounded retry: a fixed small attempt count (e.g. 3) with an injectable delay function, so tests don't sleep for real:

```rust
/// Delay function is injected so tests can substitute a no-op instead of a real sleep.
fn retry_with_delay<F, D>(attempts: u32, delay: D, mut op: F) -> Result<(), DeviceError>
where
    F: FnMut() -> Result<(), DeviceError>,
    D: Fn(),
{ /* ... */ }
```

(Exact shape is implementer's call — the requirement is: fixed bounded attempt count, not open-ended retry/backoff, delay is injectable/fakeable, and the underlying error is surfaced once the bound is exhausted.)

**Explicitly not handled here:** multi-process concurrent access (two `colorer` invocations, or another tool, touching the same device at once) — see the plan's "Explicitly Out of Scope" list. Do not add locking.

**5. `src/commands/set.rs` and CLI wiring:**

```rust
struct SetArgs { id: String, color: String }
```

Add `Commands::Set(SetArgs)` to the `Commands` enum in `src/cli.rs`. The command implementation parses `color` via `color::parse_color`, looks up the backend(s), and calls `set_color`. On `DeviceError::PermissionDenied`, print a clear "permission denied — see udev setup" message rather than a raw OS error.

**6. udev rule** — `packaging/udev/71-colorer.rules`:

```
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="<target-vid>", ATTRS{idProduct}=="<target-pid>", TAG+="uaccess"
```

Uses the modern `uaccess` tag (matches what OpenRGB ships). **Must be scoped to the specific target device's `idVendor`/`idProduct` — this scoping is required, not optional.** A blanket `KERNEL=="hidraw*"` rule would grant access to every unrelated HID device on the system (other keyboards, security keys, etc.), an unjustified privilege grant for what this milestone actually needs. Fill in the real `<target-vid>`/`<target-pid>` once the target device is known per the precondition.

Document one-time install steps in `packaging/udev/README.md` or a README section:
- Copy the rules file to `/etc/udev/rules.d/`.
- `udevadm control --reload-rules && udevadm trigger`.
- Possibly replug the device.
- Explicitly note that `TAG+="uaccess"` grants access to the logged-in seat session via systemd-logind — it is **not** a permanent standing grant independent of session state. Manual verification must check actual effective access (can the current session's user open the device now), not just "is the rule file present."

### Manual verification

- After installing the scoped udev rule, `cargo run -- set <id> ff0000` actually changes the real device's color.
- Running the same command before installing the rule produces a clear "permission denied — see udev setup" error, not a raw OS error or panic.
- Unplugging the device between `list` and `set` produces a clean `DeviceGone` error, not a crash.
- `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings` all pass clean before moving to section-06.

### Explicitly out of scope for this section

- Multi-process/concurrent device access coordination (locking).
- Exhaustive partial-transaction/protocol-level failure recovery (short writes, mid-transaction disconnects, ack semantics) beyond the bounded retry and clean error surfacing specified above — these failure modes are protocol-specific to the targeted device and can't be meaningfully specified further here.
- sysfs-based `set_color` (that's section-06).

### As actually implemented

The precondition above was not met: no real target device (make/model +
known HID report protocol) was available when this section was
implemented. Per the user's explicit choice, only the scaffolding was
built, tested against fakes, with no real device wired in. See
`docs/knowledge/device/set-color-hid.md` and
`docs/knowledge/cli/set-command.md` for the full current-behavior writeup;
summary of what differs from the plan text above:

- **Files:** matches the planned list exactly (`src/color.rs`,
  `src/device/mod.rs`, `src/device/hid.rs`, `src/cli.rs`,
  `src/commands/set.rs`, `packaging/udev/71-colorer.rules`,
  `packaging/udev/README.md`), plus one addition not in the original plan:
  `ColorWriter` (`src/device/mod.rs`), a small trait separate from
  `DeviceBackend` for the write capability, since command-layer dispatch
  needed something to iterate over — `main.rs::color_writers()`.
- **`IMPLEMENTED_PROTOCOLS` (the "explicit list of devices this code
  actually implements a protocol for" the plan calls for) is empty.**
  `set_color` therefore returns `DeviceError::Unsupported` for every real
  device today. Test #2 (report-building pure function producing expected
  bytes for the real device's protocol) is correspondingly not implemented
  — there's no real protocol to test against yet, matching the plan's own
  "fill in once the real protocol is known."
- **Revalidation-before-write is narrower than "device identity" implies.**
  It catches a device swap between `set_color_impl`'s two `discover()`
  calls (microseconds apart, within one `set` invocation) but not a device
  swapped between an earlier `list` and this `set` — `set` has no
  independently-recorded expected identity to check against, only the
  opaque `id`. Documented as a known limitation in
  `docs/knowledge/device/set-color-hid.md` rather than treated as solved.
- **`packaging/udev/71-colorer.rules`** ships with `<target-vid>`/
  `<target-pid>` placeholder tokens (not filled in), clearly marked as
  non-functional until a real device is chosen — see the file and its
  README for the install/verification steps that were still written even
  though the rule itself doesn't match anything yet.
- **Tests:** all 8 numbered test areas from "Tests first" above are
  covered except #2 (see above); `retry_with_delay`, `identity_matches`,
  and `map_hid_error` also have direct unit tests beyond what the plan
  enumerated. 8 new tests in `src/color.rs`, 13 in
  `src/device/hid.rs::tests`, 6 in `src/commands/set.rs::tests`, plus 3 CLI
  parse tests in `src/cli.rs`.