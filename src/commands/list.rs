use crate::device::vendors::known_vendor;
use crate::device::{DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource};

const EMPTY_MESSAGE: &str = "no known RGB devices found (try --all)";

/// Run the `list` command: discover via all given backends, filter (unless
/// show_all), format as a table.
pub fn run_list(
    backends: &[Box<dyn DeviceBackend>],
    show_all: bool,
) -> Result<String, DeviceError> {
    let mut devices = Vec::new();
    for backend in backends {
        devices.extend(backend.discover()?);
    }

    let filtered: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| show_all || is_allowed(d))
        .collect();

    if filtered.is_empty() {
        return Ok(EMPTY_MESSAGE.to_string());
    }

    Ok(format_table(&filtered))
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
        let capability = match d.capability {
            DeviceCapability::Unknown => "unknown",
            DeviceCapability::SingleColor => "single-color",
            DeviceCapability::MultiColor => "multi-color",
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            d.id, source, vendor, d.label, d.path, vendor_id, product_id, capability
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
mod tests {
    use super::*;
    use crate::device::make_id;

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
}
