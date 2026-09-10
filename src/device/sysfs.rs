use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::color::Rgb;
use crate::device::{
    ColorWriter, DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource, make_id,
};

/// Discovers LED-class devices under a configurable root (`/sys/class/leds`
/// in production, a temp-dir fixture in tests). Classifies each entry by
/// which attribute files are present; entries with missing partner
/// attributes, unreadable files, or that vanish mid-scan are skipped with a
/// warning rather than failing the whole scan.
pub struct SysfsBackend {
    root: PathBuf,
}

impl SysfsBackend {
    /// `root` is the directory to scan, e.g. "/sys/class/leds" in production
    /// or a temp dir in tests — the key testability seam for this backend.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        SysfsBackend { root: root.into() }
    }
}

impl DeviceBackend for SysfsBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        let entries = fs::read_dir(&self.root).map_err(DeviceError::Io)?;

        let mut devices = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("warning: skipping unreadable sysfs entry: {err}");
                    continue;
                }
            };

            let dir_path = entry.path();
            let label = entry.file_name().to_string_lossy().into_owned();

            let has_multi_intensity = dir_path.join("multi_intensity").exists();
            let has_multi_index = dir_path.join("multi_index").exists();
            let has_color = dir_path.join("color").exists();
            let has_brightness =
                dir_path.join("brightness").exists() && dir_path.join("max_brightness").exists();

            // `has_color` is checked before `has_brightness` deliberately, not per the
            // plan text's literal order: real vendor-color drivers (e.g. hid-ite8291r3)
            // typically expose brightness/max_brightness alongside `color`, so checking
            // brightness first would misclassify them as SingleColor and silently break
            // section-06's dependency on this section recording the vendor-color fact.
            // `.exists()` treats an unreadable file the same as an absent one (it
            // swallows permission errors as `false`), so a present-but-unreadable
            // attribute is indistinguishable here from a genuinely absent one — both
            // fall through to the same skip/classify paths below. Acceptable per this
            // section's own spec hedge ("or simply absent-but-expected").
            let capability = if has_multi_intensity && has_multi_index {
                DeviceCapability::MultiColor
            } else if has_multi_intensity != has_multi_index {
                eprintln!(
                    "warning: skipping {}: multi_intensity/multi_index partner attribute missing",
                    dir_path.display()
                );
                continue;
            } else if has_color {
                DeviceCapability::VendorColor
            } else if has_brightness {
                DeviceCapability::SingleColor
            } else {
                eprintln!(
                    "warning: skipping {}: no recognized LED capability attributes",
                    dir_path.display()
                );
                continue;
            };

            let path = dir_path.to_string_lossy().into_owned();
            devices.push(DeviceInfo {
                id: make_id(DeviceSource::Sysfs, &path),
                source: DeviceSource::Sysfs,
                label,
                vendor_id: None,
                product_id: None,
                interface_number: None,
                usage_page: None,
                usage: None,
                path,
                capability,
            });
        }

        Ok(devices)
    }
}

/// Channel names as they appear in `multi_index`. Kernel LED-class multi-color
/// devices don't guarantee any particular write order — `multi_index` is the
/// authoritative source, read fresh at write time (see module docs on
/// `set_color`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiChannel {
    Red,
    Green,
    Blue,
}

impl MultiChannel {
    fn parse(token: &str) -> Option<Self> {
        match token {
            "red" => Some(MultiChannel::Red),
            "green" => Some(MultiChannel::Green),
            "blue" => Some(MultiChannel::Blue),
            _ => None,
        }
    }

    fn value(self, color: Rgb) -> u8 {
        match self {
            MultiChannel::Red => color.r,
            MultiChannel::Green => color.g,
            MultiChannel::Blue => color.b,
        }
    }
}

/// Given the channel order from `multi_index` and the parallel per-channel
/// ceilings from `multi_max_intensity`, computes the values to write to
/// `multi_intensity` for `color`, in `order`. Pure — no I/O — so the scaling
/// math (notably non-255 ceilings) can be unit-tested directly.
fn scale_multi_intensity(order: &[MultiChannel], max_intensity: &[u32], color: Rgb) -> Vec<u32> {
    order
        .iter()
        .zip(max_intensity)
        // u64 intermediate: `max` comes straight from a sysfs file and isn't
        // bounded, so an 8-bit channel value times a corrupt/huge ceiling
        // must not overflow a u32 multiply.
        .map(|(channel, &max)| {
            let scaled = channel.value(color) as u64 * max as u64 / 255;
            scaled.min(u32::MAX as u64) as u32
        })
        .collect()
}

fn parse_channel_order(raw: &str) -> io::Result<Vec<MultiChannel>> {
    let order = raw
        .split_whitespace()
        .map(|token| {
            MultiChannel::parse(token).ok_or_else(|| {
                io::Error::other(format!("unrecognized multi_index channel: {token}"))
            })
        })
        .collect::<io::Result<Vec<MultiChannel>>>()?;

    let is_rgb_triplet = order.len() == 3
        && order.contains(&MultiChannel::Red)
        && order.contains(&MultiChannel::Green)
        && order.contains(&MultiChannel::Blue);
    if !is_rgb_triplet {
        return Err(io::Error::other(format!(
            "expected multi_index to name red, green, blue once each, got {raw:?}"
        )));
    }

    Ok(order)
}

fn parse_intensity_list(raw: &str, expected_len: usize) -> io::Result<Vec<u32>> {
    let values = raw
        .split_whitespace()
        .map(|token| {
            token
                .parse::<u32>()
                .map_err(|_| io::Error::other(format!("invalid intensity value: {token}")))
        })
        .collect::<io::Result<Vec<u32>>>()?;
    if values.len() != expected_len {
        return Err(io::Error::other(format!(
            "expected {expected_len} multi_max_intensity values, found {}",
            values.len()
        )));
    }
    Ok(values)
}

fn read_attr(dir: &Path, name: &str) -> Result<String, DeviceError> {
    let path = dir.join(name);
    fs::read_to_string(&path).map_err(|err| map_io_error(&path, err))
}

fn write_attr(dir: &Path, name: &str, contents: &str) -> Result<(), DeviceError> {
    let path = dir.join(name);
    fs::write(&path, contents).map_err(|err| map_io_error(&path, err))
}

fn map_io_error(path: &Path, err: io::Error) -> DeviceError {
    if err.kind() == io::ErrorKind::PermissionDenied {
        DeviceError::PermissionDenied {
            path: path.display().to_string(),
        }
    } else {
        DeviceError::Io(err)
    }
}

/// Writes `color` to a `MultiColor` device at `dir`: scales the incoming 8-bit
/// channel values against the actual `multi_max_intensity` ceilings (not
/// assumed 255), writes them to `multi_intensity` in the `multi_index` write
/// order, and also raises `brightness` to `max_brightness` — a
/// `multi_intensity` write alone is invisible while `brightness` is low/zero
/// since `led_brightness = brightness * multi_intensity / max_brightness`.
fn write_multi_color(dir: &Path, color: Rgb) -> Result<(), DeviceError> {
    let order_raw = read_attr(dir, "multi_index")?;
    let order = parse_channel_order(&order_raw).map_err(DeviceError::Io)?;

    let max_raw = read_attr(dir, "multi_max_intensity")?;
    let max_intensity = parse_intensity_list(&max_raw, order.len()).map_err(DeviceError::Io)?;

    let values = scale_multi_intensity(&order, &max_intensity, color);
    let line = values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    write_attr(dir, "multi_intensity", &line)?;

    let max_brightness = read_attr(dir, "max_brightness")?;
    write_attr(dir, "brightness", max_brightness.trim())
}

/// Writes `color` to a vendor `color`-attribute device (e.g. `hid-ite8291r3`-
/// class) as an `aabbcc`-format hex triplet.
fn write_vendor_color(dir: &Path, color: Rgb) -> Result<(), DeviceError> {
    let hex = format!("{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    write_attr(dir, "color", &hex)
}

/// True if `current`'s capability (the discovery-time proxy for "same
/// attribute set") still matches what was recorded for `original`. Sysfs
/// devices carry no vendor/product id to compare (unlike HID's
/// `identity_matches` in `hid.rs`), so a capability change is the strongest
/// available signal that the directory now holds a different or reshaped
/// device.
fn identity_matches(original: &DeviceInfo, current: &DeviceInfo) -> bool {
    original.capability == current.capability
}

/// Core `set_color` logic, independent of real discovery so it can be
/// exercised with fakes (mirrors `hid.rs`'s `set_color_impl`). Steps:
/// 1. Resolve `id` to a device via `discover` (`NotFound` if absent).
/// 2. Reject `SingleColor` (fixed-color) devices immediately as `Unsupported`
///    — before revalidating or touching any file.
/// 3. Re-run `discover` and confirm the device at `id` still exists and its
///    capability still matches what was originally discovered
///    (`DeviceGone`/`IdentityMismatch`).
/// 4. Write the color via the capability-appropriate path.
fn set_color_core(
    id: &str,
    mut discover: impl FnMut() -> Result<Vec<DeviceInfo>, DeviceError>,
    color: Rgb,
) -> Result<(), DeviceError> {
    let original = discover()?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DeviceError::NotFound { id: id.to_string() })?;

    if original.capability == DeviceCapability::SingleColor {
        return Err(DeviceError::Unsupported {
            id: id.to_string(),
            operation: "set_color",
        });
    }

    let current = discover()?.into_iter().find(|d| d.id == id);
    match current {
        None => return Err(DeviceError::DeviceGone { id: id.to_string() }),
        Some(ref c) if identity_matches(&original, c) => {}
        Some(_) => return Err(DeviceError::IdentityMismatch { id: id.to_string() }),
    }

    let dir = PathBuf::from(&original.path);
    match original.capability {
        DeviceCapability::MultiColor => write_multi_color(&dir, color),
        DeviceCapability::VendorColor => write_vendor_color(&dir, color),
        // SingleColor already returned above; Unknown is never produced by
        // SysfsBackend::discover (see mod.rs) but is handled defensively
        // since DeviceCapability is a crate-wide enum.
        DeviceCapability::SingleColor | DeviceCapability::Unknown => {
            Err(DeviceError::Unsupported {
                id: id.to_string(),
                operation: "set_color",
            })
        }
    }
}

impl ColorWriter for SysfsBackend {
    /// Sets `color` on the sysfs LED device identified by `id`. See
    /// `set_color_core` for the revalidation/dispatch logic; udev rules
    /// granting unprivileged write access to the relevant attribute files
    /// (deferred until a real `MultiColor`/`VendorColor` target device is
    /// confirmed present — see `packaging/udev/71-colorer.rules`) are a
    /// prerequisite for this to succeed outside of a root shell.
    fn set_color(&self, id: &str, color: Rgb) -> Result<(), DeviceError> {
        set_color_core(id, || self.discover(), color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn touch(dir: &std::path::Path, name: &str) {
        fs::write(dir.join(name), "0").unwrap();
    }

    fn make_entry(root: &std::path::Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn multi_intensity_and_index_present_classifies_multi_color() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "rgb0");
        touch(&dev, "multi_intensity");
        touch(&dev, "multi_index");
        touch(&dev, "multi_max_intensity");
        touch(&dev, "brightness");
        touch(&dev, "max_brightness");

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].capability, DeviceCapability::MultiColor);
        assert_eq!(found[0].source, DeviceSource::Sysfs);
    }

    #[test]
    fn brightness_only_classifies_single_color() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "led0");
        touch(&dev, "brightness");
        touch(&dev, "max_brightness");

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].capability, DeviceCapability::SingleColor);
    }

    #[test]
    fn vendor_color_file_recorded_without_multi_intensity() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "ite8291r3");
        touch(&dev, "color");

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].capability, DeviceCapability::VendorColor);
    }

    #[test]
    fn vendor_color_wins_over_brightness_when_both_present() {
        // Real vendor-color drivers (e.g. hid-ite8291r3) typically expose
        // brightness/max_brightness alongside `color` — VendorColor must still
        // be recorded, not silently downgraded to SingleColor.
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "ite8291r3");
        touch(&dev, "color");
        touch(&dev, "brightness");
        touch(&dev, "max_brightness");

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].capability, DeviceCapability::VendorColor);
    }

    #[test]
    fn missing_partner_attribute_is_skipped_not_fatal() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "broken0");
        touch(&dev, "multi_index");
        // multi_intensity deliberately absent — malformed entry.

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn zero_valid_entries_returns_empty_vec_not_error() {
        let root = tempdir().unwrap();

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn unreadable_root_returns_err() {
        let root = tempdir().unwrap();
        let missing = root.path().join("does-not-exist");

        let backend = SysfsBackend::new(missing);
        let result = backend.discover();
        assert!(matches!(result, Err(DeviceError::Io(_))));
    }

    #[test]
    fn merged_table_labels_source_as_sysfs() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "led0");
        touch(&dev, "brightness");
        touch(&dev, "max_brightness");

        let backend = SysfsBackend::new(root.path());
        let found = backend.discover().unwrap();
        assert_eq!(found[0].source, DeviceSource::Sysfs);
        assert!(found[0].vendor_id.is_none());
        assert!(found[0].product_id.is_none());
    }

    // --- scale_multi_intensity, in isolation ---

    #[test]
    fn scale_multi_intensity_identity_at_255_ceiling() {
        let order = [MultiChannel::Red, MultiChannel::Green, MultiChannel::Blue];
        let max = [255u32, 255, 255];
        let color = Rgb {
            r: 10,
            g: 20,
            b: 30,
        };
        assert_eq!(scale_multi_intensity(&order, &max, color), vec![10, 20, 30]);
    }

    #[test]
    fn scale_multi_intensity_scales_against_non_255_ceiling() {
        let order = [MultiChannel::Red, MultiChannel::Green, MultiChannel::Blue];
        let max = [15u32, 15, 15];
        let color = Rgb {
            r: 255,
            g: 128,
            b: 0,
        };
        assert_eq!(scale_multi_intensity(&order, &max, color), vec![15, 7, 0]);
    }

    // --- set_color: fixture-backed happy paths ---

    fn multi_color_fixture(root: &std::path::Path, index: &str, max_intensity: &str) -> PathBuf {
        let dev = make_entry(root, "rgb0");
        fs::write(dev.join("multi_intensity"), "0 0 0").unwrap();
        fs::write(dev.join("multi_index"), index).unwrap();
        fs::write(dev.join("multi_max_intensity"), max_intensity).unwrap();
        fs::write(dev.join("brightness"), "0").unwrap();
        fs::write(dev.join("max_brightness"), "255").unwrap();
        dev
    }

    #[test]
    fn set_color_writes_multi_intensity_in_multi_index_order() {
        let root = tempdir().unwrap();
        multi_color_fixture(root.path(), "green blue red", "255 255 255");

        let backend = SysfsBackend::new(root.path());
        let id = backend.discover().unwrap()[0].id.clone();
        let color = Rgb {
            r: 10,
            g: 20,
            b: 30,
        };

        backend.set_color(&id, color).unwrap();

        let dev = root.path().join("rgb0");
        let written = fs::read_to_string(dev.join("multi_intensity")).unwrap();
        assert_eq!(written, "20 30 10", "must follow multi_index's G,B,R order");
    }

    #[test]
    fn set_color_raises_brightness_to_usable_level() {
        let root = tempdir().unwrap();
        multi_color_fixture(root.path(), "red green blue", "255 255 255");

        let backend = SysfsBackend::new(root.path());
        let id = backend.discover().unwrap()[0].id.clone();

        backend.set_color(&id, Rgb { r: 1, g: 2, b: 3 }).unwrap();

        let dev = root.path().join("rgb0");
        let brightness = fs::read_to_string(dev.join("brightness")).unwrap();
        assert_eq!(
            brightness, "255",
            "brightness must be raised, not left at its low starting value"
        );
    }

    #[test]
    fn set_color_rejects_malformed_multi_index_channel_set() {
        // Duplicate "red" instead of a green channel: must not silently zip
        // a wrong/duplicated channel into the write, must error instead.
        let root = tempdir().unwrap();
        multi_color_fixture(root.path(), "red red blue", "255 255 255");

        let backend = SysfsBackend::new(root.path());
        let id = backend.discover().unwrap()[0].id.clone();

        let result = backend.set_color(&id, Rgb { r: 1, g: 2, b: 3 });

        assert!(matches!(result, Err(DeviceError::Io(_))));
        let dev = root.path().join("rgb0");
        let written = fs::read_to_string(dev.join("multi_intensity")).unwrap();
        assert_eq!(
            written, "0 0 0",
            "must not write on a malformed channel set"
        );
    }

    #[test]
    fn set_color_writes_vendor_color_hex() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "ite8291r3");
        fs::write(dev.join("color"), "000000").unwrap();

        let backend = SysfsBackend::new(root.path());
        let id = backend.discover().unwrap()[0].id.clone();

        backend
            .set_color(
                &id,
                Rgb {
                    r: 0xaa,
                    g: 0xbb,
                    b: 0xcc,
                },
            )
            .unwrap();

        let written = fs::read_to_string(dev.join("color")).unwrap();
        assert_eq!(written, "aabbcc");
    }

    #[test]
    fn set_color_rejects_single_color_without_writing() {
        let root = tempdir().unwrap();
        let dev = make_entry(root.path(), "led0");
        fs::write(dev.join("brightness"), "5").unwrap();
        fs::write(dev.join("max_brightness"), "255").unwrap();

        let backend = SysfsBackend::new(root.path());
        let id = backend.discover().unwrap()[0].id.clone();

        let result = backend.set_color(&id, Rgb { r: 1, g: 2, b: 3 });

        assert!(matches!(result, Err(DeviceError::Unsupported { .. })));
        let brightness = fs::read_to_string(dev.join("brightness")).unwrap();
        assert_eq!(
            brightness, "5",
            "must not write anything for a fixed-color device"
        );
    }

    // --- set_color_core: revalidation, via injected discover ---

    fn sysfs_device(path: &str, capability: DeviceCapability) -> DeviceInfo {
        DeviceInfo {
            id: make_id(DeviceSource::Sysfs, path),
            source: DeviceSource::Sysfs,
            label: "test".to_string(),
            vendor_id: None,
            product_id: None,
            interface_number: None,
            usage_page: None,
            usage: None,
            path: path.to_string(),
            capability,
        }
    }

    #[test]
    fn set_color_core_missing_id_returns_not_found() {
        let device = sysfs_device("/sys/class/leds/rgb0", DeviceCapability::MultiColor);

        let result = set_color_core(
            "does-not-exist",
            || Ok(vec![device.clone()]),
            Rgb { r: 0, g: 0, b: 0 },
        );

        assert!(matches!(result, Err(DeviceError::NotFound { .. })));
    }

    #[test]
    fn set_color_core_detects_device_gone_before_writing() {
        use std::cell::Cell;

        let device = sysfs_device("/sys/class/leds/rgb0", DeviceCapability::MultiColor);
        let id = device.id.clone();
        let call_count = Cell::new(0u32);

        let result = set_color_core(
            &id,
            || {
                let n = call_count.get();
                call_count.set(n + 1);
                if n == 0 {
                    Ok(vec![device.clone()])
                } else {
                    Ok(vec![])
                }
            },
            Rgb { r: 0, g: 0, b: 0 },
        );

        assert!(matches!(result, Err(DeviceError::DeviceGone { .. })));
    }

    #[test]
    fn set_color_core_detects_identity_mismatch_before_writing() {
        use std::cell::Cell;

        let original = sysfs_device("/sys/class/leds/rgb0", DeviceCapability::MultiColor);
        let id = original.id.clone();
        // Same id/path, different capability: simulates the directory's
        // attribute shape having changed since discovery.
        let mut reshaped = original.clone();
        reshaped.capability = DeviceCapability::VendorColor;
        let call_count = Cell::new(0u32);

        let result = set_color_core(
            &id,
            || {
                let n = call_count.get();
                call_count.set(n + 1);
                if n == 0 {
                    Ok(vec![original.clone()])
                } else {
                    Ok(vec![reshaped.clone()])
                }
            },
            Rgb { r: 0, g: 0, b: 0 },
        );

        assert!(matches!(result, Err(DeviceError::IdentityMismatch { .. })));
    }
}
