use hidapi::HidApi;

use crate::device::{
    DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource, make_id,
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
