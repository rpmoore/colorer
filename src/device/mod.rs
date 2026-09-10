use std::fmt;

pub mod hid;
pub mod sysfs;
pub mod vendors;

/// Where a device was discovered.
// Sysfs is constructed starting in section-03; only Hid is produced by this section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSource {
    Hid,
    #[allow(dead_code)]
    Sysfs,
}

/// What color control the device is known to support, from discovery alone.
/// This is a *discovery-time* guess, not a promise that `set` supports the device —
/// later milestones additionally gate `set` on an actually-implemented protocol
/// for that specific device, not just this capability tag.
// SingleColor/MultiColor/VendorColor are produced by SysfsBackend classification
// (src/device/sysfs.rs); only Unknown is constructed by the HID backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCapability {
    Unknown,
    SingleColor,
    MultiColor,
    /// Non-standard drivers (e.g. `hid-ite8291r3`-style) expose a `color`
    /// sysfs file taking a hex-triplet string instead of the standard
    /// `multi_*` scheme. Recorded at discovery time so section-06 (M5) can
    /// determine, from discovery output alone, whether a device uses this
    /// vendor attribute before attempting to write to it.
    VendorColor,
}

/// One discovered RGB-capable (or possibly-RGB-capable) device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub source: DeviceSource,
    pub label: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    // Read starting in section-05 (M4) for identity/interface revalidation before writes.
    #[allow(dead_code)]
    pub interface_number: Option<i32>,
    #[allow(dead_code)]
    pub usage_page: Option<u16>,
    #[allow(dead_code)]
    pub usage: Option<u16>,
    pub path: String,
    pub capability: DeviceCapability,
}

/// Errors surfaced by device discovery/control. Several variants are part of
/// the stable contract this section establishes but are only constructed
/// starting in later sections (Io/Hid error wrapping, PermissionDenied and
/// the show/set-specific variants) — see claude-plan.md's Core Types section.
#[derive(Debug)]
#[allow(dead_code)]
pub enum DeviceError {
    Io(std::io::Error),
    Hid(hidapi::HidError),
    PermissionDenied { path: String },
    NotFound { id: String },
    DeviceGone { id: String },
    IdentityMismatch { id: String },
    Unsupported { id: String, operation: &'static str },
}

impl fmt::Display for DeviceCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DeviceCapability::Unknown => "unknown",
            DeviceCapability::SingleColor => "single-color",
            DeviceCapability::MultiColor => "multi-color",
            DeviceCapability::VendorColor => "vendor-color",
        };
        write!(f, "{s}")
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Io(e) => write!(f, "I/O error: {e}"),
            DeviceError::Hid(e) => write!(f, "HID error: {e}"),
            DeviceError::PermissionDenied { path } => {
                write!(f, "permission denied accessing {path} — see udev setup")
            }
            DeviceError::NotFound { id } => write!(f, "device not found: {id}"),
            DeviceError::DeviceGone { id } => write!(f, "device disconnected: {id}"),
            DeviceError::IdentityMismatch { id } => {
                write!(f, "device at {id} no longer matches what was discovered")
            }
            DeviceError::Unsupported { id, operation } => {
                write!(f, "{operation} is not supported for device {id}")
            }
        }
    }
}

pub trait DeviceBackend {
    /// Discover devices this backend knows about. Must not require elevated privileges.
    fn discover(&self) -> Result<Vec<DeviceInfo>, DeviceError>;
}

/// FNV-1a: a small, well-documented, deterministic-across-Rust-versions hash.
/// Used instead of `std::collections::hash_map::DefaultHasher`, whose algorithm
/// the stdlib explicitly does not guarantee stable across Rust releases — a
/// property that matters here since `id` is meant to be a stable locator a
/// user can type into `show <id>`/`set <id>` across separate CLI invocations,
/// potentially built with different compiler versions over time.
fn fnv1a(bytes: &[u8]) -> u32 {
    const FNV_OFFSET_BASIS: u32 = 0x811c9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Derives a stable, deterministic, human-typeable id for a device from its
/// source and path. Two devices with the same source+path always get the same
/// id; distinct paths are collision-resistant via a 32-bit FNV-1a hash.
pub fn make_id(source: DeviceSource, path: &str) -> String {
    let hash = fnv1a(path.as_bytes());
    let prefix = match source {
        DeviceSource::Hid => "hid",
        DeviceSource::Sysfs => "sysfs",
    };
    format!("{prefix}-{hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_error_variants_are_distinguishable() {
        let not_found = DeviceError::NotFound {
            id: "x".to_string(),
        };
        assert!(matches!(not_found, DeviceError::NotFound { .. }));
        assert!(!matches!(not_found, DeviceError::DeviceGone { .. }));

        let gone = DeviceError::DeviceGone {
            id: "x".to_string(),
        };
        assert!(matches!(gone, DeviceError::DeviceGone { .. }));

        let mismatch = DeviceError::IdentityMismatch {
            id: "x".to_string(),
        };
        assert!(matches!(mismatch, DeviceError::IdentityMismatch { .. }));

        let denied = DeviceError::PermissionDenied {
            path: "/dev/hidraw0".to_string(),
        };
        assert!(matches!(denied, DeviceError::PermissionDenied { .. }));

        let unsupported = DeviceError::Unsupported {
            id: "x".to_string(),
            operation: "set_color",
        };
        assert!(matches!(unsupported, DeviceError::Unsupported { .. }));
    }

    #[test]
    fn device_info_allows_none_for_irrelevant_fields() {
        let sysfs_entry = DeviceInfo {
            id: make_id(DeviceSource::Sysfs, "/sys/class/leds/foo"),
            source: DeviceSource::Sysfs,
            label: "foo".to_string(),
            vendor_id: None,
            product_id: None,
            interface_number: None,
            usage_page: None,
            usage: None,
            path: "/sys/class/leds/foo".to_string(),
            capability: DeviceCapability::Unknown,
        };
        assert!(sysfs_entry.vendor_id.is_none());
        assert!(sysfs_entry.interface_number.is_none());

        let hid_entry = DeviceInfo {
            id: make_id(DeviceSource::Hid, "/dev/hidraw0"),
            source: DeviceSource::Hid,
            label: "bar".to_string(),
            vendor_id: Some(0x1b1c),
            product_id: Some(0x1),
            interface_number: Some(0),
            usage_page: Some(1),
            usage: Some(6),
            path: "/dev/hidraw0".to_string(),
            capability: DeviceCapability::Unknown,
        };
        assert!(hid_entry.vendor_id.is_some());
    }

    #[test]
    fn make_id_is_deterministic_and_source_scoped() {
        let a1 = make_id(DeviceSource::Hid, "/dev/hidraw0");
        let a2 = make_id(DeviceSource::Hid, "/dev/hidraw0");
        assert_eq!(a1, a2);

        let b = make_id(DeviceSource::Sysfs, "/dev/hidraw0");
        assert_ne!(a1, b, "same path but different source must not collide");
        assert!(a1.starts_with("hid-"));
        assert!(b.starts_with("sysfs-"));
    }

    #[test]
    fn make_id_distinguishes_distinct_paths() {
        let a = make_id(DeviceSource::Hid, "/dev/hidraw0");
        let b = make_id(DeviceSource::Hid, "/dev/hidraw1");
        assert_ne!(a, b);
    }
}
