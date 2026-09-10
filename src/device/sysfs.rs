use std::fs;
use std::path::PathBuf;

use crate::device::{
    DeviceBackend, DeviceCapability, DeviceError, DeviceInfo, DeviceSource, make_id,
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
}
