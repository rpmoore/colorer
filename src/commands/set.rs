use std::fmt;

use crate::color::{ColorParseError, parse_color};
use crate::device::{ColorWriter, DeviceError};

/// Errors surfaced by the `set` command: either the color argument itself
/// was malformed, or a device-layer error occurred (already includes a
/// clear "see udev setup" message for `PermissionDenied` — see
/// `DeviceError`'s `Display` impl).
#[derive(Debug)]
pub enum SetError {
    InvalidColor(ColorParseError),
    Device(DeviceError),
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::InvalidColor(e) => write!(f, "{e}"),
            SetError::Device(e) => write!(f, "{e}"),
        }
    }
}

/// Run the `set` command: parse `color`, then try each writer in turn for
/// one that recognizes `id`. Only one writer is expected to ever match a
/// given id (ids are backend-scoped via their `hid-`/`sysfs-` prefix), so
/// the first non-`NotFound` result wins; if every writer reports `NotFound`,
/// that's what's returned.
pub fn run_set(
    writers: &[Box<dyn ColorWriter>],
    id: &str,
    color_input: &str,
) -> Result<String, SetError> {
    let color = parse_color(color_input).map_err(SetError::InvalidColor)?;

    let mut last_not_found = None;
    for writer in writers {
        match writer.set_color(id, color) {
            Ok(()) => return Ok(format!("{id}: color set to {color_input}")),
            Err(DeviceError::NotFound { id }) => {
                last_not_found = Some(DeviceError::NotFound { id });
            }
            Err(err) => return Err(SetError::Device(err)),
        }
    }

    Err(SetError::Device(
        last_not_found.unwrap_or(DeviceError::NotFound { id: id.to_string() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgb;

    struct FakeWriter {
        matching_id: &'static str,
        result: fn(Rgb) -> Result<(), DeviceError>,
    }

    impl ColorWriter for FakeWriter {
        fn set_color(&self, id: &str, color: Rgb) -> Result<(), DeviceError> {
            if id == self.matching_id {
                (self.result)(color)
            } else {
                Err(DeviceError::NotFound { id: id.to_string() })
            }
        }
    }

    #[test]
    fn rejects_invalid_color_before_touching_any_writer() {
        struct PanicsOnCall;
        impl ColorWriter for PanicsOnCall {
            fn set_color(&self, _id: &str, _color: Rgb) -> Result<(), DeviceError> {
                unreachable!("must not be called for an invalid color")
            }
        }
        let writers: Vec<Box<dyn ColorWriter>> = vec![Box::new(PanicsOnCall)];

        let result = run_set(&writers, "hid-1", "not-a-color");
        assert!(matches!(result, Err(SetError::InvalidColor(_))));
    }

    #[test]
    fn successful_set_reports_id_and_color() {
        let writers: Vec<Box<dyn ColorWriter>> = vec![Box::new(FakeWriter {
            matching_id: "hid-1",
            result: |_color| Ok(()),
        })];

        let result = run_set(&writers, "hid-1", "ff0000").unwrap();
        assert!(result.contains("hid-1"));
        assert!(result.contains("ff0000"));
    }

    #[test]
    fn unmatched_id_falls_through_to_next_writer() {
        let writers: Vec<Box<dyn ColorWriter>> = vec![
            Box::new(FakeWriter {
                matching_id: "hid-1",
                result: |_c| Ok(()),
            }),
            Box::new(FakeWriter {
                matching_id: "sysfs-1",
                result: |_c| Ok(()),
            }),
        ];

        let result = run_set(&writers, "sysfs-1", "00ff00").unwrap();
        assert!(result.contains("sysfs-1"));
    }

    #[test]
    fn id_matching_no_writer_returns_not_found() {
        let writers: Vec<Box<dyn ColorWriter>> = vec![Box::new(FakeWriter {
            matching_id: "hid-1",
            result: |_c| Ok(()),
        })];

        let result = run_set(&writers, "does-not-exist", "00ff00");
        assert!(matches!(
            result,
            Err(SetError::Device(DeviceError::NotFound { .. }))
        ));
    }

    #[test]
    fn device_error_from_matching_writer_propagates() {
        let writers: Vec<Box<dyn ColorWriter>> = vec![Box::new(FakeWriter {
            matching_id: "hid-1",
            result: |_c| {
                Err(DeviceError::Unsupported {
                    id: "hid-1".to_string(),
                    operation: "set_color",
                })
            },
        })];

        let result = run_set(&writers, "hid-1", "00ff00");
        assert!(matches!(
            result,
            Err(SetError::Device(DeviceError::Unsupported { .. }))
        ));
    }

    #[test]
    fn permission_denied_message_mentions_udev() {
        let writers: Vec<Box<dyn ColorWriter>> = vec![Box::new(FakeWriter {
            matching_id: "hid-1",
            result: |_c| {
                Err(DeviceError::PermissionDenied {
                    path: "/dev/hidraw0".to_string(),
                })
            },
        })];

        let result = run_set(&writers, "hid-1", "00ff00").unwrap_err();
        assert!(result.to_string().contains("udev"));
    }
}
