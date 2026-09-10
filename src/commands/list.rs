use crate::device::vendors::known_vendor;
use crate::device::{DeviceBackend, DeviceError, DeviceInfo, DeviceSource};

const EMPTY_MESSAGE: &str = "no known RGB devices found (try --all)";

/// Run the `list` command: discover via all given backends, filter (unless
/// show_all), format as a table.
///
/// Partial-failure policy: if one backend's `discover()` errors, its results
/// are simply omitted (with a warning line) rather than failing the whole
/// command — a backend being unreadable (e.g. no `/sys/class/leds`) must not
/// sink another backend's results.
pub fn run_list(
    backends: &[Box<dyn DeviceBackend>],
    show_all: bool,
) -> Result<String, DeviceError> {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    for backend in backends {
        match backend.discover() {
            Ok(found) => devices.extend(found),
            Err(err) => warnings.push(format!("warning: backend discovery failed: {err}")),
        }
    }

    let filtered: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| show_all || is_allowed(d))
        .collect();

    let mut out = String::new();
    for warning in &warnings {
        out.push_str(warning);
        out.push('\n');
    }

    if filtered.is_empty() {
        out.push_str(EMPTY_MESSAGE);
        return Ok(out);
    }

    out.push_str(&format_table(&filtered));
    Ok(out)
}

/// Vendor-allowlist filtering applies only to HID-sourced entries; sysfs
/// entries have no vendor ID and are never subject to vendor filtering.
fn is_allowed(device: &DeviceInfo) -> bool {
    match device.source {
        DeviceSource::Hid => device
            .vendor_id
            .is_some_and(|vid| known_vendor(vid).is_some()),
        DeviceSource::Sysfs => true,
    }
}

fn format_table(devices: &[&DeviceInfo]) -> String {
    let mut out =
        String::from("id\tsource\tvendor\tproduct\tpath\tvendor_id\tproduct_id\tcapability\n");
    for d in devices {
        let source = match d.source {
            DeviceSource::Hid => "hid",
            DeviceSource::Sysfs => "sysfs",
        };
        let vendor = d.vendor_id.and_then(known_vendor).unwrap_or("");
        let vendor_id = d.vendor_id.map(|v| format!("{v:04x}")).unwrap_or_default();
        let product_id = d.product_id.map(|v| format!("{v:04x}")).unwrap_or_default();
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            d.id, source, vendor, d.label, d.path, vendor_id, product_id, d.capability
        ));
    }
    out
}

#[cfg(test)]
pub(crate) struct FakeBackend {
    pub devices: Vec<DeviceInfo>,
}

#[cfg(test)]
impl DeviceBackend for FakeBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        Ok(self.devices.clone())
    }
}

#[cfg(test)]
struct FailingBackend;

#[cfg(test)]
impl DeviceBackend for FailingBackend {
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError> {
        Err(DeviceError::Io(std::io::Error::other(
            "simulated backend failure",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceCapability, make_id};

    fn hid_device(label: &str, path: &str, vendor_id: u16) -> DeviceInfo {
        DeviceInfo {
            id: make_id(DeviceSource::Hid, path),
            source: DeviceSource::Hid,
            label: label.to_string(),
            vendor_id: Some(vendor_id),
            product_id: Some(0x0001),
            interface_number: Some(0),
            usage_page: Some(1),
            usage: Some(6),
            path: path.to_string(),
            capability: DeviceCapability::Unknown,
        }
    }

    fn sysfs_device(label: &str, path: &str) -> DeviceInfo {
        DeviceInfo {
            id: make_id(DeviceSource::Sysfs, path),
            source: DeviceSource::Sysfs,
            label: label.to_string(),
            vendor_id: None,
            product_id: None,
            interface_number: None,
            usage_page: None,
            usage: None,
            path: path.to_string(),
            capability: DeviceCapability::SingleColor,
        }
    }

    #[test]
    fn run_list_filters_to_allowlist_by_default() {
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![
                hid_device("Corsair Keyboard", "/dev/hidraw0", 0x1b1c),
                hid_device("Unknown Device", "/dev/hidraw1", 0xffff),
            ],
        });
        let backends = vec![backend];

        let filtered = run_list(&backends, false).unwrap();
        assert!(filtered.contains("Corsair Keyboard"));
        assert!(!filtered.contains("Unknown Device"));

        let all = run_list(&backends, true).unwrap();
        assert!(all.contains("Corsair Keyboard"));
        assert!(all.contains("Unknown Device"));
    }

    #[test]
    fn run_list_empty_result_prints_clear_message() {
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![hid_device("Unknown Device", "/dev/hidraw0", 0xffff)],
        });
        let backends = vec![backend];

        let result = run_list(&backends, false).unwrap();
        assert_eq!(result, EMPTY_MESSAGE);
    }

    #[test]
    fn run_list_table_includes_all_columns() {
        let backend: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![hid_device("Corsair Keyboard", "/dev/hidraw0", 0x1b1c)],
        });
        let backends = vec![backend];

        let result = run_list(&backends, false).unwrap();
        for expected in [
            "source",
            "vendor",
            "product",
            "path",
            "vendor_id",
            "product_id",
            "capability",
            "hid",
            "Corsair",
            "Corsair Keyboard",
            "/dev/hidraw0",
            "1b1c",
            "0001",
            "unknown",
        ] {
            assert!(
                result.contains(expected),
                "missing {expected:?} in {result:?}"
            );
        }
    }

    #[test]
    fn distinct_devices_get_distinct_ids() {
        let a = hid_device("Corsair Keyboard", "/dev/hidraw0", 0x1b1c);
        let b = hid_device("Corsair Keyboard", "/dev/hidraw1", 0x1b1c);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn merged_table_labels_each_row_source() {
        let hid: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![hid_device("Corsair Keyboard", "/dev/hidraw0", 0x1b1c)],
        });
        let sysfs: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![sysfs_device("rgb0", "/sys/class/leds/rgb0")],
        });
        let backends = vec![hid, sysfs];

        let result = run_list(&backends, true).unwrap();
        assert!(result.contains("\thid\t"));
        assert!(result.contains("\tsysfs\t"));
    }

    #[test]
    fn sysfs_backend_error_still_returns_hid_results_with_warning() {
        let hid: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![hid_device("Corsair Keyboard", "/dev/hidraw0", 0x1b1c)],
        });
        let failing: Box<dyn DeviceBackend> = Box::new(FailingBackend);
        let backends = vec![hid, failing];

        let result = run_list(&backends, false).unwrap();
        assert!(result.contains("Corsair Keyboard"));
        assert!(result.contains("warning:"));
    }

    #[test]
    fn hid_backend_error_still_returns_sysfs_results_with_warning() {
        let failing: Box<dyn DeviceBackend> = Box::new(FailingBackend);
        let sysfs: Box<dyn DeviceBackend> = Box::new(FakeBackend {
            devices: vec![sysfs_device("rgb0", "/sys/class/leds/rgb0")],
        });
        let backends = vec![failing, sysfs];

        let result = run_list(&backends, false).unwrap();
        assert!(result.contains("rgb0"));
        assert!(result.contains("warning:"));
    }
}
