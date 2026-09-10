use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::device::{DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource};

/// Run the `show` command: discover across all given backends, find the
/// device matching `id`, and format its full detail. Returns
/// `DeviceError::NotFound` if no discovered device has this id.
///
/// Applies the same partial-failure tolerance as `list` (section-03): a
/// backend whose `discover()` errors is skipped rather than propagated — a
/// match is still searched for among the surviving backends' results. Unlike
/// `list` (which always returns `Ok` and can embed a warning line in its
/// output string), `show`'s `Err(NotFound)` path has no string body to carry
/// a warning in, so a failing backend is reported via `eprintln!` instead.
pub fn run_show(backends: &[Box<dyn DeviceBackend>], id: &str) -> Result<String, DeviceError> {
    let mut devices = Vec::new();
    for backend in backends {
        match backend.discover() {
            Ok(found) => devices.extend(found),
            Err(err) => eprintln!("warning: backend discovery failed: {err}"),
        }
    }

    // `make_id` prefixes ids by source (hid-/sysfs-), and production only
    // registers one instance of each backend type, so a collision here would
    // require two same-source backends producing the same path — not
    // expected in practice. `.find()` takes the first match if it ever did.
    let device = devices
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DeviceError::NotFound { id: id.to_string() })?;

    Ok(format_detail(&device))
}

fn format_detail(d: &DeviceInfo) -> String {
    let source = match d.source {
        DeviceSource::Hid => "hid",
        DeviceSource::Sysfs => "sysfs",
    };

    let mut out = String::new();
    let _ = writeln!(out, "id: {}", d.id);
    let _ = writeln!(out, "label: {}", d.label);
    let _ = writeln!(out, "source: {source}");
    let _ = writeln!(out, "path: {}", d.path);
    let _ = writeln!(out, "capability: {}", d.capability);

    // HID-only fields: only printed when Some, never as misleading zero/empty
    // values for sysfs-sourced entries where they're legitimately absent.
    if let Some(vendor_id) = d.vendor_id {
        let _ = writeln!(out, "vendor_id: {vendor_id:04x}");
    }
    if let Some(product_id) = d.product_id {
        let _ = writeln!(out, "product_id: {product_id:04x}");
    }
    if let Some(interface_number) = d.interface_number {
        let _ = writeln!(out, "interface_number: {interface_number}");
    }
    if let Some(usage_page) = d.usage_page {
        let _ = writeln!(out, "usage_page: {usage_page:#06x}");
    }
    if let Some(usage) = d.usage {
        let _ = writeln!(out, "usage: {usage:#06x}");
    }

    let _ = writeln!(out, "current: {}", current_state(d));

    out
}

/// Current color/brightness where readable without elevated privileges.
/// HID has no read path yet (post-M4 concern) — always "unknown" there.
/// Sysfs attribute files are plain-readable; missing/unreadable/empty falls
/// back to "unknown" rather than erroring, consistent with `show` being a
/// read-only, best-effort detail command.
fn current_state(d: &DeviceInfo) -> String {
    if d.source != DeviceSource::Sysfs {
        return "unknown".to_string();
    }

    let attr = match d.capability {
        DeviceCapability::MultiColor => "multi_intensity",
        DeviceCapability::VendorColor => "color",
        DeviceCapability::SingleColor | DeviceCapability::Unknown => "brightness",
    };

    fs::read_to_string(Path::new(&d.path).join(attr))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::list::FakeBackend;
    use crate::device::make_id;
    use tempfile::tempdir;

    struct FailingBackend;

    impl DeviceBackend for FailingBackend {
        fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
            Err(DeviceError::Io(std::io::Error::other(
                "simulated backend failure",
            )))
        }
    }

    fn hid_device(path: &str) -> DeviceInfo {
        DeviceInfo {
            id: make_id(DeviceSource::Hid, path),
            source: DeviceSource::Hid,
            label: "Test HID".to_string(),
            vendor_id: Some(0x1b1c),
            product_id: Some(0x0001),
            interface_number: Some(0),
            usage_page: Some(1),
            usage: Some(6),
            path: path.to_string(),
            capability: DeviceCapability::Unknown,
        }
    }

    #[test]
    fn matching_id_returns_full_detail() {
        let device = hid_device("/dev/hidraw0");
        let expected_id = device.id.clone();
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![device],
        });
        let backends = vec![backend];

        let result = run_show(&backends, &expected_id).unwrap();
        for expected in [
            "source: hid",
            "path: /dev/hidraw0",
            "vendor_id: 1b1c",
            "product_id: 0001",
            "interface_number: 0",
            "usage_page: 0x0001",
            "usage: 0x0006",
            "capability: unknown",
        ] {
            assert!(
                result.contains(expected),
                "missing {expected:?} in {result:?}"
            );
        }
    }

    #[test]
    fn nonmatching_id_returns_not_found() {
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![hid_device("/dev/hidraw0")],
        });
        let backends = vec![backend];

        let result = run_show(&backends, "does-not-exist");
        assert!(matches!(result, Err(DeviceError::NotFound { .. })));
    }

    #[test]
    fn does_not_panic_on_empty_id() {
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend { devices: vec![] });
        let backends = vec![backend];

        let result = run_show(&backends, "");
        assert!(matches!(result, Err(DeviceError::NotFound { .. })));
    }

    #[test]
    fn sysfs_device_reads_current_brightness() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("brightness"), "128\n").unwrap();
        let path = root.path().to_string_lossy().into_owned();

        let device = DeviceInfo {
            id: make_id(DeviceSource::Sysfs, &path),
            source: DeviceSource::Sysfs,
            label: "led0".to_string(),
            vendor_id: None,
            product_id: None,
            interface_number: None,
            usage_page: None,
            usage: None,
            path: path.clone(),
            capability: DeviceCapability::SingleColor,
        };
        let expected_id = device.id.clone();
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![device],
        });
        let backends = vec![backend];

        let result = run_show(&backends, &expected_id).unwrap();
        assert!(result.contains("current: 128"));
    }

    #[test]
    fn sysfs_multi_color_device_reads_multi_intensity() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("multi_intensity"), "10 20 30\n").unwrap();
        let path = root.path().to_string_lossy().into_owned();

        let device = DeviceInfo {
            id: make_id(DeviceSource::Sysfs, &path),
            source: DeviceSource::Sysfs,
            label: "rgb0".to_string(),
            vendor_id: None,
            product_id: None,
            interface_number: None,
            usage_page: None,
            usage: None,
            path: path.clone(),
            capability: DeviceCapability::MultiColor,
        };
        let expected_id = device.id.clone();
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![device],
        });
        let backends = vec![backend];

        let result = run_show(&backends, &expected_id).unwrap();
        assert!(result.contains("current: 10 20 30"));
    }

    #[test]
    fn sysfs_failure_does_not_block_hid_match() {
        let device = hid_device("/dev/hidraw0");
        let expected_id = device.id.clone();
        let hid: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![device],
        });
        let failing: Box<dyn DeviceBackend> = Box::new(FailingBackend);
        let backends = vec![hid, failing];

        let result = run_show(&backends, &expected_id).unwrap();
        assert!(result.contains("source: hid"));
    }

    #[test]
    fn failing_backend_alone_still_returns_not_found() {
        let failing: Box<dyn DeviceBackend> = Box::new(FailingBackend);
        let backends = vec![failing];

        let result = run_show(&backends, "any-id");
        assert!(matches!(result, Err(DeviceError::NotFound { .. })));
    }

    #[test]
    fn hid_device_shows_unknown_current_state() {
        let device = hid_device("/dev/hidraw0");
        let expected_id = device.id.clone();
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![device],
        });
        let backends = vec![backend];

        let result = run_show(&backends, &expected_id).unwrap();
        assert!(result.contains("current: unknown"));
    }
}
