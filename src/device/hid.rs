use std::ffi::CString;
use std::io::ErrorKind;

use hidapi::{HidApi, HidError};

use crate::color::Rgb;
use crate::device::{
    ColorWriter, DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource, make_id,
};

/// Discovers USB HID devices via `hidapi`. Always returns every HID device it
/// sees, unfiltered — vendor-allowlist filtering is entirely a
/// `commands/list.rs` concern, never this backend's.
///
/// Note: `hidapi` enumerates one entry per HID *interface*, not one per
/// physical device — a single mouse/keyboard with multiple interfaces (e.g.
/// separate HID report descriptors for standard input vs. vendor-specific
/// control) shows up as multiple `DeviceInfo` rows sharing the same
/// vendor/product id but different `path`/`interface_number`. This was
/// confirmed empirically against real hardware during this section (the
/// plan's own "Device identity note" flagged this as an open question).
/// Deliberately not deduplicated here — no dedup policy has been decided yet
/// (see `docs/knowledge/device/index.md`); resolve it when section-05/-06
/// need to target one specific interface for writes.
pub struct HidBackend;

impl HidBackend {
    pub fn new() -> Self {
        HidBackend
    }
}

impl Default for HidBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceBackend for HidBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        let api = HidApi::new().map_err(DeviceError::Hid)?;
        let devices = api
            .device_list()
            .map(|info| {
                let path = info.path().to_string_lossy().into_owned();
                let label = info
                    .product_string()
                    .unwrap_or("Unknown HID device")
                    .to_string();
                DeviceInfo {
                    id: make_id(DeviceSource::Hid, &path),
                    source: DeviceSource::Hid,
                    label,
                    vendor_id: Some(info.vendor_id()),
                    product_id: Some(info.product_id()),
                    interface_number: Some(info.interface_number()),
                    usage_page: Some(info.usage_page()),
                    usage: Some(info.usage()),
                    path,
                    capability: DeviceCapability::Unknown,
                }
            })
            .collect();
        Ok(devices)
    }
}

/// Abstracts the raw HID write so it can be faked in tests without a real
/// device handle. Only used within this module (production `RealHidTransport`
/// plus test fakes) — not part of the crate's public API.
///
/// Two write paths exist because devices disagree on which one their color
/// protocol uses: `write_report` is a plain interrupt OUT report;
/// `write_feature_report` is a control-transfer Feature report (`HIDIOCSFEATURE`)
/// — e.g. Razer's Chroma protocol (see `razer_ornata_v3_static_report`) requires
/// the latter. `ReportKind` on each `IMPLEMENTED_PROTOCOLS` entry picks which one
/// `set_color_impl` calls for that device.
trait HidTransport {
    fn write_report(&self, report: &[u8]) -> Result<(), DeviceError>;
    fn write_feature_report(&self, report: &[u8]) -> Result<(), DeviceError>;
}

/// Wraps an already-opened `hidapi::HidDevice`. Production-only; never
/// constructed in tests, which use a fake `HidTransport` instead.
struct RealHidTransport(hidapi::HidDevice);

impl HidTransport for RealHidTransport {
    fn write_report(&self, report: &[u8]) -> Result<(), DeviceError> {
        self.0
            .write(report)
            .map(|_written| ())
            .map_err(map_hid_error)
    }

    fn write_feature_report(&self, report: &[u8]) -> Result<(), DeviceError> {
        self.0.send_feature_report(report).map_err(map_hid_error)
    }
}

/// Best-effort classification of a `hidapi` error: a permission-denied I/O
/// error is surfaced distinctly (so command-layer code can point at udev
/// setup) rather than conflated with other I/O/HID errors.
fn map_hid_error(err: HidError) -> DeviceError {
    if let HidError::IoError { error } = &err
        && error.kind() == ErrorKind::PermissionDenied
    {
        return DeviceError::PermissionDenied {
            path: error.to_string(),
        };
    }
    DeviceError::Hid(err)
}

/// Builds the outgoing HID report(s) for a color, in send order. Most
/// protocols are a single report (e.g. Razer), but some (e.g. Gigabyte RGB
/// Fusion 2: set-effect then apply) need a short fixed sequence sent one
/// after another — a `Vec` rather than a single report keeps that without a
/// second `ReportKind`/builder type. Kept as a plain function pointer (not a
/// closure) so both production and tests can pass it around without
/// capturing state.
type ReportBuilder = fn(&Rgb) -> Vec<Vec<u8>>;

/// Which write path a device's protocol uses — see `HidTransport`'s doc comment.
// Output is unused by any current IMPLEMENTED_PROTOCOLS entry (the only real
// device so far, Ornata V3, uses Feature) but is part of the stable contract
// for a future device whose protocol uses plain interrupt writes instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportKind {
    #[allow(dead_code)]
    Output,
    Feature,
}

/// Devices this backend has an actually-implemented, working color protocol
/// for, keyed by (vendor_id, product_id, interface_number). `interface_number`
/// is part of the key, not just vendor/product: a single physical device like
/// the Razer Ornata V3 enumerates as several `DeviceInfo` rows (one per HID
/// interface, see `docs/knowledge/device/hid-interface-enumeration.md`)
/// sharing one vendor/product id, but only one specific interface accepts
/// this command protocol — matching on vendor/product alone would attempt
/// the same report on interfaces that don't understand it.
///
/// Razer Ornata V3 (`1532:02a1`, interface 2): confirmed against
/// `openrazer/openrazer`'s `razerkbd_driver.c` (`USB_DEVICE_ID_RAZER_ORNATA_V3`
/// dispatches to `razer_chroma_extended_matrix_effect_static` with
/// `transaction_id = 0x1F`) — see `razer_ornata_v3_static_report`.
///
/// Razer Naga X (`1532:0096`, interface 3): confirmed against
/// `openrazer/openrazer`'s `razermouse_driver.c`
/// (`USB_DEVICE_ID_RAZER_NAGA_X` in `matrix_effect_static_common`'s dispatch
/// also routes to `razer_chroma_extended_matrix_effect_static`, same
/// `transaction_id = 0x1F`) — see `razer_naga_x_static_report`. Sets both of
/// this mouse's zones (scroll wheel, side underglow) since this CLI has no
/// per-zone addressing.
///
/// Gigabyte RGB Fusion 2 onboard controller (`048d:5711`, an ITE IT5711 chip,
/// interface 1) — specifically its CPU-area ARGB strip header only, on one
/// specific motherboard model: confirmed against `OpenRGB`'s
/// `Controllers/GigabyteRGBFusion2USBController` plus live hardware
/// verification — see `gigabyte_fusion2_cpu_strip_report`'s doc comment for
/// the full scope caveat.
///
/// Being in `vendors.rs`'s known-RGB-vendor allowlist (used by `list`) does
/// NOT imply an entry here — `list` support and `set` support are
/// independent gates.
const IMPLEMENTED_PROTOCOLS: &[(u16, u16, i32, ReportKind, ReportBuilder)] = &[
    (
        0x1532,
        0x02a1,
        2,
        ReportKind::Feature,
        razer_ornata_v3_static_report,
    ),
    (
        0x1532,
        0x0096,
        3,
        ReportKind::Feature,
        razer_naga_x_static_report,
    ),
    (
        0x048d,
        0x5711,
        1,
        ReportKind::Feature,
        gigabyte_fusion2_cpu_strip_report,
    ),
];

fn implemented_protocol(
    vendor_id: Option<u16>,
    product_id: Option<u16>,
    interface_number: Option<i32>,
) -> Option<(ReportKind, ReportBuilder)> {
    let (vid, pid, iface) = (vendor_id?, product_id?, interface_number?);
    IMPLEMENTED_PROTOCOLS
        .iter()
        .find(|(v, p, i, _, _)| *v == vid && *p == pid && *i == iface)
        .map(|(_, _, _, kind, builder)| (*kind, *builder))
}

/// Length of a Razer "Chroma" control report — fixed across the whole
/// protocol family (status, transaction id, remaining-packet count,
/// protocol type, data size, command class/id, 80 argument bytes, crc,
/// reserved). See `openrazer/openrazer`'s `driver/razercommon.h`
/// `struct razer_report`.
const RAZER_REPORT_LEN: usize = 90;

/// Razer's report checksum: XOR of every byte from `remaining_packets`
/// through the end of `arguments` (offsets 2..88), excluding the leading
/// status/transaction-id bytes and the trailing crc/reserved bytes
/// themselves. Mirrors `razer_calculate_crc` in `openrazer/openrazer`'s
/// `driver/razercommon.c`.
fn razer_crc(report: &[u8; RAZER_REPORT_LEN]) -> u8 {
    report[2..88].iter().fold(0u8, |crc, &b| crc ^ b)
}

/// Builds the 90-byte Razer "Chroma" `razer_report` struct for
/// SET_LED_MATRIX_EFFECT / static-color: command class `0x0F`/id `0x02`,
/// targeting `VARSTORE`/the given `led_id`, effect `STATIC` (`0x01`), with
/// `arguments[5] = 0x01` (present in every observed capture but otherwise
/// unexplained by upstream) followed by the RGB bytes. Pure — no framing —
/// see e.g. `razer_ornata_v3_static_report` for an actual `ReportBuilder`
/// sent over the wire.
///
/// Shared by every device/zone using this exact command shape —
/// `razerkbd_driver.c`'s `matrix_effect_static` dispatch (Ornata V3,
/// `led_id = BACKLIGHT_LED`) and `razermouse_driver.c`'s per-zone
/// `matrix_effect_static_common` dispatch (Naga X, `led_id` =
/// `SCROLL_WHEEL_LED`/`LEFT_SIDE_LED`) both route here with the same
/// `transaction_id = 0x1F` — only `led_id` differs.
fn razer_chroma_extended_matrix_effect_static_struct(
    led_id: u8,
    color: &Rgb,
) -> [u8; RAZER_REPORT_LEN] {
    const VARSTORE: u8 = 0x01;
    const STATIC_EFFECT: u8 = 0x01;
    const TRANSACTION_ID: u8 = 0x1f;
    const COMMAND_CLASS: u8 = 0x0f;
    const COMMAND_ID: u8 = 0x02;
    const DATA_SIZE: u8 = 9;

    let mut report = [0u8; RAZER_REPORT_LEN];
    report[1] = TRANSACTION_ID;
    report[5] = DATA_SIZE;
    report[6] = COMMAND_CLASS;
    report[7] = COMMAND_ID;
    report[8] = VARSTORE;
    report[9] = led_id;
    report[10] = STATIC_EFFECT;
    report[13] = 0x01;
    report[14] = color.r;
    report[15] = color.g;
    report[16] = color.b;
    report[88] = razer_crc(&report);
    report
}

/// `razer_chroma_extended_matrix_effect_static_struct` targeting the Ornata
/// V3's `BACKLIGHT_LED` zone (the keyboard's whole backlight — its only
/// zone this protocol addresses).
fn razer_ornata_v3_static_struct(color: &Rgb) -> [u8; RAZER_REPORT_LEN] {
    const BACKLIGHT_LED: u8 = 0x05;
    razer_chroma_extended_matrix_effect_static_struct(BACKLIGHT_LED, color)
}

/// The actual `ReportBuilder` for the Ornata V3, sent as a Feature report
/// (`ReportKind::Feature`). Prefixes the 90-byte `razer_report` struct with an
/// explicit `0x00` HID report-ID byte (91 bytes total) — confirmed
/// empirically against real hardware, not documented anywhere upstream:
/// `hidapi`'s Feature-report calls (`send_feature_report`/`get_feature_report`)
/// require this prefix even for a device that doesn't number its reports
/// (no `Report ID` tag in its descriptor). Without it, `send_feature_report`
/// still returns `Ok`, but a `get_feature_report` readback shows the
/// device's `status` byte stuck at `0x00` (unprocessed) and nothing visibly
/// changes; with the prefix, `status` comes back `0x02` (success) and the
/// keyboard's color actually updates. The kernel driver this protocol is
/// reverse-engineered from doesn't need this prefix because it issues a raw
/// `usb_control_msg` rather than going through `hidraw`'s Feature-report
/// ioctls.
fn razer_ornata_v3_static_report(color: &Rgb) -> Vec<Vec<u8>> {
    vec![razer_report_wire_bytes(&razer_ornata_v3_static_struct(
        color,
    ))]
}

/// Prefixes a 90-byte `razer_report` struct with the empirically-required
/// `0x00` HID report-ID byte — see `razer_ornata_v3_static_report`'s doc
/// comment for why. Shared by every Razer Chroma device/zone report built
/// on this struct.
fn razer_report_wire_bytes(body: &[u8; RAZER_REPORT_LEN]) -> Vec<u8> {
    let mut report = Vec::with_capacity(RAZER_REPORT_LEN + 1);
    report.push(0x00);
    report.extend_from_slice(body);
    report
}

/// The `ReportBuilder` for the Razer Naga X: two independent zones, both set
/// to `color` in one `set_color` call since this CLI has no per-zone
/// addressing — `SCROLL_WHEEL_LED` (the scroll wheel) and `LEFT_SIDE_LED`
/// (the underglow strip near the thumb buttons, this mouse's primary visible
/// RGB — it has no logo/backlight zone, per `razermouse_driver.c`'s Naga X
/// `CREATE_DEVICE_FILE` block). Both use the same
/// `razer_chroma_extended_matrix_effect_static_struct` shape as the Ornata
/// V3; `razermouse_driver.c`'s per-LED `matrix_effect_static_common`
/// dispatch routes `USB_DEVICE_ID_RAZER_NAGA_X` to
/// `razer_chroma_extended_matrix_effect_static` with the same
/// `transaction_id = 0x1F`.
fn razer_naga_x_static_report(color: &Rgb) -> Vec<Vec<u8>> {
    const SCROLL_WHEEL_LED: u8 = 0x01;
    const LEFT_SIDE_LED: u8 = 0x11;

    vec![
        razer_report_wire_bytes(&razer_chroma_extended_matrix_effect_static_struct(
            SCROLL_WHEEL_LED,
            color,
        )),
        razer_report_wire_bytes(&razer_chroma_extended_matrix_effect_static_struct(
            LEFT_SIDE_LED,
            color,
        )),
    ]
}

/// Length of a Gigabyte RGB Fusion 2 (ITE IT5711-family) control report —
/// includes the leading report-ID byte, matching the device's own HID
/// descriptor (`Report ID 0xCC`, report size 8 bits × count 0x3F = 63 data
/// bytes + 1 ID byte = 64). See `OpenRGB`'s
/// `GigabyteRGBFusion2USBController.h`'s `FUSION2_USB_BUFFER_SIZE`.
const GIGABYTE_REPORT_LEN: usize = 64;

/// The report ID this controller's Feature reports are numbered under —
/// present as an explicit `Report ID` tag in the device's HID descriptor
/// (confirmed against the real `hidraw` report descriptor), unlike the
/// Ornata V3's unnumbered reports. Because it's a genuine numbered report,
/// the buffer sent is exactly `GIGABYTE_REPORT_LEN` bytes with this as
/// byte 0 — no extra framing prefix needed (contrast
/// `razer_ornata_v3_static_report`'s doc comment).
const GIGABYTE_REPORT_ID: u8 = 0xcc;

/// This is a `RGBFUSION2_57XX_LEDS_MAX`-addressable "Gen2" ARGB strip
/// (`OpenRGB`'s `SupportsGen2`/`ScanGen2Strips`), not a simple fixed-color
/// hardware "effect" zone: an earlier implementation tried the generic
/// `PktEffect`/`EFFECT_STATIC` command used by `OpenRGB`'s "Motherboard LEDs
/// always use effect mode" comment, but on this board that command had no
/// visible effect at all — direct per-LED "Direct" mode
/// (`SetStripColors`/`PktRGB`) is what actually works, confirmed live.
///
/// Scoped to exactly one header on one motherboard model — **not** a
/// general Gigabyte RGB Fusion 2 implementation:
/// - Board: Gigabyte X870E AORUS PRO (`048d:5711`, ITE IT5711 chip,
///   `OpenRGB`'s `it5711_11_device` layout).
/// - Header: `HDR_D_LED2` / zone "ARGB_V2_2" (the CPU-area ARGB strip) —
///   the only one of this board's 6 zones (3 "Linear" ARGB strips + 3
///   "Single" LEDs: `LED_C`/`IO Cover`/`Chipset Accent`) that was actually
///   probed and hardware-confirmed. `colorer set` on this device therefore
///   only changes the CPU strip's color — other zones (case fans,
///   motherboard accent lighting, etc.) are untouched.
///
/// Two per-board-model facts baked in here, confirmed empirically rather
/// than assumed, since both are known to vary by board/header and getting
/// either wrong silently scrambles the LED colors instead of erroring:
/// - **LED count (48)**: from a live `OpenRGB`-protocol "Gen2" strip scan
///   (`GEN2_LED_BASE_SCAN`-based scan/detect handshake) against this exact
///   header — not read dynamically here (`ReportBuilder` is a pure
///   `fn(&Rgb) -> Vec<Vec<u8>>`, no read-back capability in `HidTransport`
///   yet; scanning would need one).
/// - **Channel byte order (`GRB`)**: from this device's own calibration
///   register (`cal_strip1` in the info-report readback, decoded per
///   `OpenRGB`'s `DecodeCalibrationBuffer`) — not the naive
///   `RGBToBGRColor`-packed order the earlier (non-working) effect-based
///   attempt used. Verified by sending pure red and observing pure red.
///
/// # A real behavioral hazard hit while reverse-engineering this
///
/// `SetStripBuiltinEffectState`'s `enable` parameter is inverted from what
/// its name suggests: `enable = true` **re-enables** the firmware's
/// autonomous built-in effect (clears the header's bit in the "disabled"
/// bitmask register, command `0x32`); `enable = false` is what disables it
/// so the host can drive raw per-LED colors instead. Sending the bitmask
/// for *every* header with the disable meaning (as an earlier debug attempt
/// did, intending to silence a strobing default effect before setting a
/// static color) disables the built-in effect renderer everywhere at
/// once — including for the zones the caller never intended to touch — and
/// visibly turned off lighting across the whole board until a
/// `0x32`-with-zero (re-enable everything) command was sent. This function's
/// own disable command only *names* the one header it writes to
/// (`DISABLE_BUILTIN_BIT_D_LED2`) — but see the caveat below, it's still not
/// a fully safe operation with respect to other headers' state.
///
/// # Known gaps, accepted rather than solved here
///
/// - **Not a read-modify-write.** The disable-bitmask register (command
///   `0x32`) is written as an *absolute* byte containing only this header's
///   bit, not merged with the register's actual current value — `HidTransport`
///   has no read-back capability, so there's nothing to merge with. If some
///   other header's disable bit was set by something else (concurrent
///   `colorer` use targeting a different header once one exists, or another
///   RGB tool), this call incidentally clears it, re-enabling that header's
///   built-in effect as a side effect. Low-probability on a single-user
///   machine running only this tool against only this one header, but a real
///   gap, not a solved one.
/// - **Partial-sequence failure has no rollback.** If the disable-builtin
///   report succeeds but a later report in the sequence (an LED chunk, or
///   apply) fails on every retry, `set_color_impl` returns an error but the
///   device is left with this header's built-in effect disabled and no
///   static color ever applied — i.e. dark, not merely "unchanged" — until a
///   later successful `set_color` call (or a manual re-enable) fixes it.
fn gigabyte_fusion2_cpu_strip_report(color: &Rgb) -> Vec<Vec<u8>> {
    /// `HDR_D_LED2`'s bit in the built-in-effect-disable bitmask register
    /// (command `0x32`) — see `SetStripBuiltinEffectState`'s `switch(hdr)`.
    const DISABLE_BUILTIN_BIT_D_LED2: u8 = 0x02;
    /// `HDR_D_LED2_ARGB` — the header byte `PktRGB::Init` maps `HDR_D_LED2`
    /// to for addressable-strip writes.
    const HEADER_D_LED2_ARGB: u8 = 0x59;
    const NUM_LEDS: usize = 48;
    /// Max LEDs per `PktRGB` packet: `sizeof(leds[19])` in `OpenRGB`'s
    /// `PktRGB::RGBData`.
    const LEDS_PER_PACKET: usize = 19;
    /// This board's calibrated channel order for `HDR_D_LED2` (`cal_strip1`
    /// decoded as `"GRB"`): byte-within-LED offsets for each channel.
    const CHANNEL_OFFSET_R: usize = 1;
    const CHANNEL_OFFSET_G: usize = 0;
    const CHANNEL_OFFSET_B: usize = 2;

    fn new_report(command: u8) -> [u8; GIGABYTE_REPORT_LEN] {
        let mut report = [0u8; GIGABYTE_REPORT_LEN];
        report[0] = GIGABYTE_REPORT_ID;
        report[1] = command;
        report
    }

    let mut reports = Vec::with_capacity(1 + NUM_LEDS.div_ceil(LEDS_PER_PACKET) + 1);

    let mut disable_builtin = new_report(0x32);
    disable_builtin[2] = DISABLE_BUILTIN_BIT_D_LED2;
    reports.push(disable_builtin.to_vec());

    let led_indices: Vec<usize> = (0..NUM_LEDS).collect();
    for chunk in led_indices.chunks(LEDS_PER_PACKET) {
        let sent = chunk[0];
        let byte_count = chunk.len() * 3;
        debug_assert!(
            byte_count <= u8::MAX as usize,
            "PktRGB's bcount field is a single byte; LEDS_PER_PACKET must stay small enough"
        );

        let mut report = new_report(HEADER_D_LED2_ARGB);
        report[2..4].copy_from_slice(&((sent * 3) as u16).to_le_bytes());
        report[4] = byte_count as u8;
        for i in 0..chunk.len() {
            let base = 5 + i * 3;
            report[base + CHANNEL_OFFSET_R] = color.r;
            report[base + CHANNEL_OFFSET_G] = color.g;
            report[base + CHANNEL_OFFSET_B] = color.b;
        }
        reports.push(report.to_vec());
    }

    let mut apply = new_report(0x28);
    apply[2] = 0xff;
    apply[3] = 0x07;
    reports.push(apply.to_vec());

    reports
}

/// Number of open/write attempts before giving up. Fixed and small: this
/// only exists to absorb the brief window where udev hasn't finished
/// applying rules to a just-plugged device yet, not to paper over real
/// failures.
const RETRY_ATTEMPTS: u32 = 3;

/// Retries `op` up to `attempts` times, calling `delay` between attempts
/// (never after the last one). `delay` is injected so tests can substitute a
/// no-op instead of sleeping for real. Surfaces the last error once the
/// bound is exhausted.
fn retry_with_delay<F, D>(attempts: u32, delay: D, mut op: F) -> Result<(), DeviceError>
where
    F: FnMut() -> Result<(), DeviceError>,
    D: Fn(),
{
    assert!(attempts >= 1, "attempts must be at least 1");
    let mut last_err = None;
    for attempt in 0..attempts {
        match op() {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < attempts {
                    delay();
                }
            }
        }
    }
    Err(last_err.expect("loop runs at least once since attempts >= 1"))
}

/// True if `current`'s identity fields still match what was recorded for
/// `original` at discovery time.
fn identity_matches(original: &DeviceInfo, current: &DeviceInfo) -> bool {
    original.vendor_id == current.vendor_id
        && original.product_id == current.product_id
        && original.interface_number == current.interface_number
}

/// Core `set_color` logic, independent of real HID I/O so it can be
/// exercised with fakes. Steps:
/// 1. Resolve `id` to a device via `discover` (`NotFound` if absent).
/// 2. Check an implemented protocol exists for that device (`Unsupported` if
///    not) — checked before the second `discover` since it's the cheaper
///    gate and avoids a second enumeration for devices we could never write
///    to anyway.
/// 3. Re-run `discover` and confirm the device at `id` still exists and still
///    matches its originally-discovered identity (`DeviceGone`/`IdentityMismatch`).
/// 4. Build the report sequence and write each one in order via
///    `open_transport`, retried (from the start of the sequence) up to
///    `attempts` times with `delay` between attempts.
///
/// Scope note: both `discover` calls happen within this single `set`
/// invocation, so this only catches a device swap happening between them
/// (microseconds apart) — e.g. a race with another process re-plugging
/// hardware mid-call. It does NOT protect against the coarser race of
/// "device seen in an earlier `list` was unplugged and replaced by a
/// different device before this `set` even started": `set` only receives an
/// opaque `id`, with no independent record of what `list` last saw at that
/// id to compare against. Closing that gap would need `set` to accept and
/// verify caller-supplied identity (e.g. an expected vendor/product id), a
/// larger contract change not part of this section.
#[allow(clippy::too_many_arguments)]
fn set_color_impl(
    id: &str,
    mut discover: impl FnMut() -> Result<Vec<DeviceInfo>, DeviceError>,
    resolve_protocol: impl FnOnce(&DeviceInfo) -> Option<(ReportKind, ReportBuilder)>,
    color: &Rgb,
    mut open_transport: impl FnMut(&DeviceInfo) -> Result<Box<dyn HidTransport>, DeviceError>,
    attempts: u32,
    delay: impl Fn(),
) -> Result<(), DeviceError> {
    let original = discover()?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DeviceError::NotFound { id: id.to_string() })?;

    let (kind, build_report) =
        resolve_protocol(&original).ok_or_else(|| DeviceError::Unsupported {
            id: id.to_string(),
            operation: "set_color",
        })?;

    let current = discover()?.into_iter().find(|d| d.id == id);
    match current {
        None => return Err(DeviceError::DeviceGone { id: id.to_string() }),
        Some(ref c) if identity_matches(&original, c) => {}
        Some(_) => return Err(DeviceError::IdentityMismatch { id: id.to_string() }),
    }

    let reports = build_report(color);
    retry_with_delay(attempts, delay, || {
        let transport = open_transport(&original)?;
        for report in &reports {
            match kind {
                ReportKind::Output => transport.write_report(report),
                ReportKind::Feature => transport.write_feature_report(report),
            }?;
        }
        Ok(())
    })
}

/// Opens the real device at `info.path` for writing.
fn open_real_transport(info: &DeviceInfo) -> Result<Box<dyn HidTransport>, DeviceError> {
    let api = HidApi::new().map_err(DeviceError::Hid)?;
    let path = CString::new(info.path.clone()).map_err(|_| {
        DeviceError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "device path contains an embedded NUL byte",
        ))
    })?;
    let device = api.open_path(&path).map_err(map_hid_error)?;
    Ok(Box::new(RealHidTransport(device)))
}

impl ColorWriter for HidBackend {
    /// Sets `color` on the HID device identified by `id`. `IMPLEMENTED_PROTOCOLS`
    /// currently has two real entries (Razer Ornata V3, Gigabyte RGB Fusion 2's
    /// CPU strip); every other device returns `DeviceError::Unsupported`. The
    /// surrounding machinery (revalidation, retry, transport abstraction) is
    /// fully built and tested so adding another real device is a matter of
    /// populating that table.
    fn set_color(&self, id: &str, color: Rgb) -> Result<(), DeviceError> {
        set_color_impl(
            id,
            || self.discover(),
            |device| {
                implemented_protocol(device.vendor_id, device.product_id, device.interface_number)
            },
            &color,
            open_real_transport,
            RETRY_ATTEMPTS,
            || std::thread::sleep(std::time::Duration::from_millis(50)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn hid_device(
        path: &str,
        vendor_id: u16,
        product_id: u16,
        interface_number: i32,
    ) -> DeviceInfo {
        DeviceInfo {
            id: make_id(DeviceSource::Hid, path),
            source: DeviceSource::Hid,
            label: "Test HID".to_string(),
            vendor_id: Some(vendor_id),
            product_id: Some(product_id),
            interface_number: Some(interface_number),
            usage_page: Some(1),
            usage: Some(6),
            path: path.to_string(),
            capability: DeviceCapability::Unknown,
        }
    }

    fn fake_report(color: &Rgb) -> Vec<Vec<u8>> {
        vec![vec![0xaa, color.r, color.g, color.b]]
    }

    struct RecordingTransport {
        calls: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    impl HidTransport for RecordingTransport {
        fn write_report(&self, report: &[u8]) -> Result<(), DeviceError> {
            self.calls.borrow_mut().push(report.to_vec());
            Ok(())
        }

        fn write_feature_report(&self, report: &[u8]) -> Result<(), DeviceError> {
            self.calls.borrow_mut().push(report.to_vec());
            Ok(())
        }
    }

    /// Unlike `RecordingTransport` (which records bytes but not which method
    /// was called), this records *which* `HidTransport` method fired —
    /// needed to verify `set_color_impl`'s `ReportKind` dispatch actually
    /// routes to the right one.
    struct KindRecordingTransport {
        kinds: Rc<RefCell<Vec<ReportKind>>>,
    }

    impl HidTransport for KindRecordingTransport {
        fn write_report(&self, _report: &[u8]) -> Result<(), DeviceError> {
            self.kinds.borrow_mut().push(ReportKind::Output);
            Ok(())
        }

        fn write_feature_report(&self, _report: &[u8]) -> Result<(), DeviceError> {
            self.kinds.borrow_mut().push(ReportKind::Feature);
            Ok(())
        }
    }

    fn marker_error() -> DeviceError {
        DeviceError::Hid(HidError::HidApiErrorEmpty)
    }

    // --- retry_with_delay, in isolation ---

    #[test]
    fn retry_succeeds_within_bound() {
        let calls = Cell::new(0u32);
        let delays = Cell::new(0u32);
        let result = retry_with_delay(
            3,
            || delays.set(delays.get() + 1),
            || {
                let n = calls.get();
                calls.set(n + 1);
                if n < 2 { Err(marker_error()) } else { Ok(()) }
            },
        );
        assert!(result.is_ok());
        assert_eq!(calls.get(), 3, "op called until it succeeds");
        assert_eq!(delays.get(), 2, "delay called between attempts only");
    }

    #[test]
    fn retry_is_bounded_and_surfaces_last_error() {
        let calls = Cell::new(0u32);
        let delays = Cell::new(0u32);
        let result = retry_with_delay(
            3,
            || delays.set(delays.get() + 1),
            || {
                calls.set(calls.get() + 1);
                Err::<(), _>(marker_error())
            },
        );
        assert!(matches!(result, Err(DeviceError::Hid(_))));
        assert_eq!(calls.get(), 3, "op is not retried past the bound");
        assert_eq!(delays.get(), 2, "no delay after the final failed attempt");
    }

    // --- set_color_impl ---

    #[test]
    fn set_color_happy_path_writes_expected_bytes() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_for_transport = calls.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 1, g: 2, b: 3 },
            move |_info| {
                Ok(Box::new(RecordingTransport {
                    calls: calls_for_transport.clone(),
                }) as Box<dyn HidTransport>)
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(result.is_ok());
        assert_eq!(calls.borrow().as_slice(), &[vec![0xaa, 1, 2, 3]]);
    }

    #[test]
    fn set_color_dispatches_feature_kind_to_write_feature_report() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let kinds_for_transport = kinds.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Feature, fake_report as ReportBuilder)),
            &Rgb { r: 1, g: 2, b: 3 },
            move |_info| {
                Ok(Box::new(KindRecordingTransport {
                    kinds: kinds_for_transport.clone(),
                }) as Box<dyn HidTransport>)
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(result.is_ok());
        assert_eq!(kinds.borrow().as_slice(), &[ReportKind::Feature]);
    }

    #[test]
    fn set_color_dispatches_output_kind_to_write_report() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let kinds_for_transport = kinds.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 1, g: 2, b: 3 },
            move |_info| {
                Ok(Box::new(KindRecordingTransport {
                    kinds: kinds_for_transport.clone(),
                }) as Box<dyn HidTransport>)
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(result.is_ok());
        assert_eq!(kinds.borrow().as_slice(), &[ReportKind::Output]);
    }

    fn multi_report(color: &Rgb) -> Vec<Vec<u8>> {
        vec![
            vec![0x01, color.r],
            vec![0x02, color.g],
            vec![0x03, color.b],
        ]
    }

    #[test]
    fn set_color_sends_a_multi_report_builder_s_reports_in_order() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_for_transport = calls.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Feature, multi_report as ReportBuilder)),
            &Rgb { r: 9, g: 8, b: 7 },
            move |_info| {
                Ok(Box::new(RecordingTransport {
                    calls: calls_for_transport.clone(),
                }) as Box<dyn HidTransport>)
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(result.is_ok());
        assert_eq!(
            calls.borrow().as_slice(),
            &[vec![0x01, 9], vec![0x02, 8], vec![0x03, 7]],
            "all reports sent, in builder order"
        );
    }

    #[test]
    fn set_color_retries_the_whole_sequence_on_a_mid_sequence_failure() {
        // A failure on the *second* report of a 3-report sequence must not
        // resume from report 2 on retry — the whole sequence restarts from
        // report 1, so the device never receives report 2 or 3 without
        // report 1 immediately before them in the same attempt.
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_for_transport = calls.clone();
        let call_count = Rc::new(Cell::new(0u32));
        let call_count_for_transport = call_count.clone();

        struct FlakyOnSecondCallTransport {
            calls: Rc<RefCell<Vec<Vec<u8>>>>,
            call_count: Rc<Cell<u32>>,
        }

        impl HidTransport for FlakyOnSecondCallTransport {
            fn write_report(&self, _report: &[u8]) -> Result<(), DeviceError> {
                unreachable!("this test only uses Feature reports")
            }

            fn write_feature_report(&self, report: &[u8]) -> Result<(), DeviceError> {
                let n = self.call_count.get();
                self.call_count.set(n + 1);
                if n == 1 {
                    return Err(marker_error());
                }
                self.calls.borrow_mut().push(report.to_vec());
                Ok(())
            }
        }

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Feature, multi_report as ReportBuilder)),
            &Rgb { r: 9, g: 8, b: 7 },
            move |_info| {
                Ok(Box::new(FlakyOnSecondCallTransport {
                    calls: calls_for_transport.clone(),
                    call_count: call_count_for_transport.clone(),
                }) as Box<dyn HidTransport>)
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(result.is_ok());
        assert_eq!(
            calls.borrow().as_slice(),
            &[
                vec![0x01, 9], // attempt 1: report 1 succeeds...
                // ...then report 2 fails, aborting attempt 1 (not recorded)
                vec![0x01, 9], // attempt 2: report 1 is *resent*, not skipped
                vec![0x02, 8],
                vec![0x03, 7],
            ],
            "a failed attempt's already-sent reports are resent on retry, \
             not resumed from the failure point"
        );
    }

    #[test]
    fn set_color_unsupported_device_is_rejected() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| None,
            &Rgb { r: 0, g: 0, b: 0 },
            |_info| unreachable!("must not attempt to open a transport for an unsupported device"),
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::Unsupported { .. })));
    }

    #[test]
    fn set_color_missing_id_returns_not_found() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);

        let result = set_color_impl(
            "does-not-exist",
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 0, g: 0, b: 0 },
            |_info| unreachable!("must not attempt to open a transport when the id is unknown"),
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::NotFound { .. })));
    }

    #[test]
    fn set_color_detects_identity_mismatch_before_writing() {
        let original = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = original.id.clone();
        // Same id, different vendor_id: simulates a different device now
        // occupying the same path/id.
        let mut swapped = original.clone();
        swapped.vendor_id = Some(0x9999);
        let call_count = Cell::new(0u32);

        let result = set_color_impl(
            &id,
            || {
                let n = call_count.get();
                call_count.set(n + 1);
                if n == 0 {
                    Ok(vec![original.clone()])
                } else {
                    Ok(vec![swapped.clone()])
                }
            },
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 0, g: 0, b: 0 },
            |_info| unreachable!("must not write once identity has changed"),
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::IdentityMismatch { .. })));
    }

    #[test]
    fn set_color_detects_device_gone_before_writing() {
        let original = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = original.id.clone();
        let call_count = Cell::new(0u32);

        let result = set_color_impl(
            &id,
            || {
                let n = call_count.get();
                call_count.set(n + 1);
                if n == 0 {
                    Ok(vec![original.clone()])
                } else {
                    Ok(vec![])
                }
            },
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 0, g: 0, b: 0 },
            |_info| unreachable!("must not write once the device is gone"),
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::DeviceGone { .. })));
    }

    #[test]
    fn set_color_retries_transient_open_failures_then_succeeds() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_for_transport = calls.clone();
        let attempt = Cell::new(0u32);
        let delays = Cell::new(0u32);

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 9, g: 9, b: 9 },
            move |_info| {
                let n = attempt.get();
                attempt.set(n + 1);
                if n < 2 {
                    Err(marker_error())
                } else {
                    Ok(Box::new(RecordingTransport {
                        calls: calls_for_transport.clone(),
                    }) as Box<dyn HidTransport>)
                }
            },
            RETRY_ATTEMPTS,
            || delays.set(delays.get() + 1),
        );

        assert!(result.is_ok());
        assert_eq!(calls.borrow().as_slice(), &[vec![0xaa, 9, 9, 9]]);
        assert_eq!(delays.get(), 2);
    }

    #[test]
    fn set_color_exhausts_retry_and_surfaces_underlying_error() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();
        let attempt = Rc::new(Cell::new(0u32));
        let attempt_for_open = attempt.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 0, g: 0, b: 0 },
            move |_info| {
                attempt_for_open.set(attempt_for_open.get() + 1);
                Err(marker_error())
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::Hid(_))));
        assert_eq!(attempt.get(), RETRY_ATTEMPTS);
    }

    #[test]
    fn set_color_surfaces_permission_denied_distinctly() {
        let device = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let id = device.id.clone();

        let result = set_color_impl(
            &id,
            || Ok(vec![device.clone()]),
            |_d| Some((ReportKind::Output, fake_report as ReportBuilder)),
            &Rgb { r: 0, g: 0, b: 0 },
            |_info| {
                Err(DeviceError::PermissionDenied {
                    path: "/dev/hidraw0".to_string(),
                })
            },
            RETRY_ATTEMPTS,
            || {},
        );

        assert!(matches!(result, Err(DeviceError::PermissionDenied { .. })));
    }

    // --- supporting helpers ---

    #[test]
    fn implemented_protocol_has_no_entry_for_unknown_device() {
        assert!(implemented_protocol(Some(0x1b1c), Some(0x1), Some(0)).is_none());
        assert!(implemented_protocol(None, Some(0x1), Some(0)).is_none());
        assert!(implemented_protocol(Some(0x1b1c), None, Some(0)).is_none());
        assert!(implemented_protocol(Some(0x1b1c), Some(0x1), None).is_none());
    }

    #[test]
    fn implemented_protocol_matches_ornata_v3_only_on_its_specific_interface() {
        let (kind, _builder) = implemented_protocol(Some(0x1532), Some(0x02a1), Some(2))
            .expect("Ornata V3 interface 2 must resolve to a protocol");
        assert_eq!(kind, ReportKind::Feature);

        // Same vendor/product but a different interface (e.g. the plain
        // keyboard boot interface) must not match — only interface 2 speaks
        // this command protocol.
        assert!(implemented_protocol(Some(0x1532), Some(0x02a1), Some(0)).is_none());
    }

    #[test]
    fn razer_ornata_v3_static_struct_matches_known_captures() {
        // Byte layout confirmed against openrazer's
        // razer_chroma_extended_matrix_effect_static doc comment: e.g.
        // "010501000001ff0000" for pure red (arguments[0..8]).
        let color = Rgb {
            r: 0xff,
            g: 0x00,
            b: 0x00,
        };
        let report = razer_ornata_v3_static_struct(&color);

        assert_eq!(report[1], 0x1f, "transaction_id");
        assert_eq!(report[5], 9, "data_size");
        assert_eq!(report[6], 0x0f, "command_class");
        assert_eq!(report[7], 0x02, "command_id");
        assert_eq!(
            &report[8..17],
            &[0x01, 0x05, 0x01, 0x00, 0x00, 0x01, 0xff, 0x00, 0x00],
            "arguments: varstore, backlight_led, static effect, pad, pad, 0x01, r, g, b"
        );
        // Hardcoded, not re-derived via razer_crc (that would be tautological
        // since razer_crc built this same byte during construction): XOR of
        // 0x09^0x0f^0x02^0x01^0x05^0x01^0x01^0xff (data_size, command_class,
        // command_id, varstore, backlight_led, static_effect, arg[5]=0x01, r)
        // over the crc range [2..88) — every other byte in range is zero.
        assert_eq!(report[88], 0xff);
    }

    #[test]
    fn razer_ornata_v3_static_report_prefixes_report_id_byte() {
        // Confirmed empirically against real hardware (see the function's
        // doc comment): hidapi's Feature-report calls need this leading
        // 0x00 even though the device doesn't number its reports.
        let color = Rgb { r: 1, g: 2, b: 3 };
        let reports = razer_ornata_v3_static_report(&color);
        let body = razer_ornata_v3_static_struct(&color);

        assert_eq!(reports.len(), 1, "single-report protocol");
        let wire = &reports[0];
        assert_eq!(wire.len(), RAZER_REPORT_LEN + 1);
        assert_eq!(wire[0], 0x00, "leading HID report-id byte");
        assert_eq!(&wire[1..], &body);
    }

    #[test]
    fn razer_naga_x_static_report_sets_scroll_wheel_and_left_side_zones() {
        // Wire byte indices are the pure struct's index + 1, since byte 0 is
        // the prepended HID report-id prefix (see
        // razer_ornata_v3_static_report_prefixes_report_id_byte).
        let color = Rgb {
            r: 0x10,
            g: 0x20,
            b: 0x30,
        };
        let reports = razer_naga_x_static_report(&color);

        assert_eq!(reports.len(), 2, "scroll wheel zone, then left-side zone");
        for report in &reports {
            assert_eq!(report.len(), RAZER_REPORT_LEN + 1);
            assert_eq!(
                report[0], 0x00,
                "leading HID report-id byte, same as Ornata V3"
            );
            assert_eq!(report[2], 0x1f, "transaction_id");
            assert_eq!(
                &report[6..9],
                &[9, 0x0f, 0x02],
                "data_size, command_class, command_id"
            );
            assert_eq!(report[9], 0x01, "varstore");
            assert_eq!(report[11], 0x01, "static effect");
            assert_eq!(&report[15..18], &[color.r, color.g, color.b]);
        }

        assert_eq!(
            reports[0][10], 0x01,
            "scroll wheel: SCROLL_WHEEL_LED led_id"
        );
        assert_eq!(reports[1][10], 0x11, "left side: LEFT_SIDE_LED led_id");
    }

    #[test]
    fn gigabyte_fusion2_cpu_strip_report_sends_disable_chunks_then_apply() {
        let color = Rgb {
            r: 0xff,
            g: 0x7f,
            b: 0x00,
        };
        let reports = gigabyte_fusion2_cpu_strip_report(&color);

        // disable-builtin, 3 LED-write chunks (19+19+10=48), apply
        assert_eq!(reports.len(), 5);
        for report in &reports {
            assert_eq!(report.len(), GIGABYTE_REPORT_LEN);
            assert_eq!(report[0], GIGABYTE_REPORT_ID);
        }

        let disable = &reports[0];
        assert_eq!(
            &disable[1..3],
            &[0x32, 0x02],
            "disable built-in effect, HDR_D_LED2's bit only"
        );

        let chunk_sizes: Vec<u8> = reports[1..4].iter().map(|r| r[4]).collect();
        assert_eq!(
            chunk_sizes,
            vec![19 * 3, 19 * 3, 10 * 3],
            "48 LEDs chunked at 19 per packet: bcount = led_count * 3"
        );

        let first_chunk = &reports[1];
        assert_eq!(first_chunk[1], 0x59, "header: HDR_D_LED2_ARGB");
        assert_eq!(
            &first_chunk[2..4],
            &[0, 0],
            "boffset: first chunk starts at 0"
        );
        assert_eq!(
            &first_chunk[5..8],
            &[color.g, color.r, color.b],
            "first LED's 3 bytes in this board's calibrated GRB order"
        );

        let second_chunk = &reports[2];
        assert_eq!(
            &second_chunk[2..4],
            &(19u16 * 3).to_le_bytes(),
            "boffset: second chunk starts after 19 LEDs * 3 bytes"
        );

        let apply = &reports[4];
        assert_eq!(&apply[1..4], &[0x28, 0xff, 0x07]);
    }

    #[test]
    fn identity_matches_compares_vendor_product_and_interface() {
        let a = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        let b = hid_device("/dev/hidraw0", 0x1234, 0x5678, 0);
        assert!(identity_matches(&a, &b));

        let mut c = a.clone();
        c.interface_number = Some(1);
        assert!(!identity_matches(&a, &c));
    }

    #[test]
    fn map_hid_error_distinguishes_permission_denied() {
        let denied = HidError::IoError {
            error: std::io::Error::new(ErrorKind::PermissionDenied, "denied"),
        };
        assert!(matches!(
            map_hid_error(denied),
            DeviceError::PermissionDenied { .. }
        ));

        let other = HidError::IoError {
            error: std::io::Error::other("boom"),
        };
        assert!(matches!(map_hid_error(other), DeviceError::Hid(_)));
    }
}
