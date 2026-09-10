use std::fmt;

/// An RGB color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Why a color string failed to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorParseError {
    /// The hex portion (after an optional leading `#`) wasn't exactly 6 characters.
    WrongLength { input: String, hex_len: usize },
    /// The hex portion contained a non-hex-digit character (including a
    /// misplaced/extra `#`).
    InvalidHex { input: String },
}

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorParseError::WrongLength { input, hex_len } => write!(
                f,
                "invalid color {input:?}: expected 6 hex digits (optionally prefixed with '#'), got {hex_len}"
            ),
            ColorParseError::InvalidHex { input } => {
                write!(
                    f,
                    "invalid color {input:?}: contains a non-hex-digit character"
                )
            }
        }
    }
}

/// Parses a color string in `"#RRGGBB"` or `"RRGGBB"` form.
pub fn parse_color(input: &str) -> Result<Rgb, ColorParseError> {
    let hex = input.strip_prefix('#').unwrap_or(input);

    if hex.len() != 6 {
        return Err(ColorParseError::WrongLength {
            input: input.to_string(),
            hex_len: hex.len(),
        });
    }

    let byte = |slice: &str| {
        u8::from_str_radix(slice, 16).map_err(|_| ColorParseError::InvalidHex {
            input: input.to_string(),
        })
    };

    Ok(Rgb {
        r: byte(&hex[0..2])?,
        g: byte(&hex[2..4])?,
        b: byte(&hex[4..6])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hash_prefixed_form() {
        assert_eq!(
            parse_color("#ff00aa").unwrap(),
            Rgb {
                r: 0xff,
                g: 0x00,
                b: 0xaa
            }
        );
    }

    #[test]
    fn parses_bare_form() {
        assert_eq!(
            parse_color("00ff00").unwrap(),
            Rgb {
                r: 0x00,
                g: 0xff,
                b: 0x00
            }
        );
    }

    #[test]
    fn parses_uppercase_hex() {
        assert_eq!(
            parse_color("#FF00AA").unwrap(),
            Rgb {
                r: 0xff,
                g: 0x00,
                b: 0xaa
            }
        );
    }

    #[test]
    fn rejects_wrong_length_too_short() {
        assert!(matches!(
            parse_color("#ff00a"),
            Err(ColorParseError::WrongLength { .. })
        ));
    }

    #[test]
    fn rejects_wrong_length_too_long() {
        assert!(matches!(
            parse_color("#ff00aabb"),
            Err(ColorParseError::WrongLength { .. })
        ));
    }

    #[test]
    fn rejects_non_hex_characters() {
        assert!(matches!(
            parse_color("#gg00aa"),
            Err(ColorParseError::InvalidHex { .. })
        ));
    }

    #[test]
    fn rejects_hash_embedded_mid_string() {
        // Correct length (6 chars after the leading strip), but the '#' is
        // in the wrong place, so it lands in the hex body as an invalid digit.
        assert!(matches!(
            parse_color("ff#0aa"),
            Err(ColorParseError::InvalidHex { .. })
        ));
    }

    #[test]
    fn rejects_missing_and_extra_hash() {
        assert!(matches!(
            parse_color("ff00a"),
            Err(ColorParseError::WrongLength { .. })
        ));
        assert!(matches!(
            parse_color("##ff00aa"),
            Err(ColorParseError::WrongLength { .. })
        ));
    }
}
