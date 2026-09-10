/// Seed list of known RGB peripheral vendor IDs. Expand as real target devices
/// are identified — this is a starting point, not exhaustive.
const KNOWN_VENDORS: &[(&str, u16)] = &[
    ("Corsair", 0x1b1c),
    ("Razer", 0x1532),
    ("Logitech", 0x046d),
    ("SteelSeries", 0x1038),
    ("ASUS", 0x0b05),
];

/// Returns the vendor name if `vendor_id` is in the known-RGB allowlist.
pub fn known_vendor(vendor_id: u16) -> Option<&'static str> {
    KNOWN_VENDORS
        .iter()
        .find(|(_, vid)| *vid == vendor_id)
        .map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vendor_returns_name_for_each_seeded_id() {
        for (name, vid) in KNOWN_VENDORS {
            assert_eq!(known_vendor(*vid), Some(*name));
        }
    }

    #[test]
    fn known_vendor_returns_none_for_unknown_id() {
        assert_eq!(known_vendor(0xffff), None);
    }
}
