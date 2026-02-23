use wdk_sys::GUID;

#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct Guid(GUID);

impl PartialEq for Guid {
    fn eq(&self, other: &Self) -> bool {
        self.0.Data1 == other.0.Data1
            && self.0.Data2 == other.0.Data2
            && self.0.Data3 == other.0.Data3
            && self.0.Data4 == other.0.Data4
    }
}

impl Eq for Guid {}

impl Guid {
    /// Creates a `Guid` from raw components.
    pub const fn from_values(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Guid(GUID {
            Data1: data1,
            Data2: data2,
            Data3: data3,
            Data4: data4,
        })
    }

    /// Parses a GUID from a string of 32 hex digits. Dashes are allowed
    /// anywhere in the string and are ignored.
    ///
    /// Hex digits are parsed in a single pass, writing directly into the
    /// GUID fields (`Data1`, `Data2`, `Data3`, `Data4`) one nibble at a time.
    ///
    /// # Examples
    ///
    /// ```
    /// # use wdf::Guid;
    /// let guid = Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").unwrap();
    /// assert_eq!(guid, Guid::from_values(
    ///     0x2aa02ab1, 0xc26e, 0x431b,
    ///     [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
    /// ));
    /// ```
    pub const fn parse(guid_str: &str) -> Result<Self, &'static str> {
        const ERR: &str = "Invalid GUID format: expected 32 hex digits, with optional dashes";

        let bytes = guid_str.as_bytes();

        // GUID fields we build up incrementally as we scan hex digits.
        let mut data1: u32 = 0;
        let mut data2: u16 = 0;
        let mut data3: u16 = 0;
        let mut data4 = [0u8; 8];

        // `count` tracks how many hex digits we've consumed so far (excluding
        // dashes). It determines which GUID field the current digit belongs to:
        //   digits  0.. 7 → Data1 (u32, 8 hex digits)
        //   digits  8..11 → Data2 (u16, 4 hex digits)
        //   digits 12..15 → Data3 (u16, 4 hex digits)
        //   digits 16..31 → Data4 ([u8; 8], 16 hex digits, 2 per byte)
        let mut count: usize = 0;
        let mut i = 0;

        while i < bytes.len() {
            let b = bytes[i];
            match b {
                // Dashes are simply skipped
                b'-' => {}
                b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                    // More than 32 hex digits is invalid
                    if count >= 32 {
                        return Err(ERR);
                    }
                    let val = Self::hex_digit_to_value(b);

                    // Shift the target field left by 4 bits and OR in the new
                    // nibble. For Data4 bytes, `(count - 16) / 2` gives the
                    // byte index; each byte accumulates two nibbles the same
                    // way (high nibble first, then low).
                    if count < 8 {
                        data1 = (data1 << 4) | val as u32;
                    } else if count < 12 {
                        data2 = (data2 << 4) | val as u16;
                    } else if count < 16 {
                        data3 = (data3 << 4) | val as u16;
                    } else {
                        let byte_idx = (count - 16) / 2;
                        data4[byte_idx] = (data4[byte_idx] << 4) | val;
                    }
                    count += 1;
                }
                // Any character that isn't a hex digit or dash is invalid.
                _ => return Err(ERR),
            }
            i += 1;
        }

        // Make sure we have seen exactly 32 hex digits.
        if count != 32 {
            return Err(ERR);
        }

        Ok(Guid(GUID {
            Data1: data1,
            Data2: data2,
            Data3: data3,
            Data4: data4,
        }))
    }

    /// Converts a single hex ASCII byte to its numeric value (0..15).
    /// Caller must ensure `digit` is a valid hex digit.
    const fn hex_digit_to_value(digit: u8) -> u8 {
        match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => unreachable!(),
        }
    }

    /// Returns the underlying raw `GUID` by value.
    pub fn to_raw(&self) -> GUID {
        self.0
    }

    pub fn as_lpcguid(&self) -> *const GUID {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_format() {
        let guid = Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").unwrap();
        assert_guid(
            &guid,
            0x2aa02ab1,
            0xc26e,
            0x431b,
            [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
        );
    }

    #[test]
    fn parse_no_dashes() {
        let guid = Guid::parse("2aa02ab1c26e431b8efe85ee8de102e4").unwrap();
        assert_guid(
            &guid,
            0x2aa02ab1,
            0xc26e,
            0x431b,
            [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
        );
    }

    #[test]
    fn parse_uppercase() {
        let guid = Guid::parse("2AA02AB1-C26E-431B-8EFE-85EE8DE102E4").unwrap();
        let lower = Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").unwrap();
        assert_eq!(guid, lower);
    }

    #[test]
    fn parse_mixed_case() {
        let guid = Guid::parse("2aA02Ab1-c26E-431b-8EfE-85eE8De102e4").unwrap();
        assert_guid(
            &guid,
            0x2aa02ab1,
            0xc26e,
            0x431b,
            [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
        );
    }

    #[test]
    fn parse_all_zeros() {
        let guid = Guid::parse("00000000-0000-0000-0000-000000000000").unwrap();
        assert_guid(&guid, 0, 0, 0, [0; 8]);
    }

    #[test]
    fn parse_all_fs() {
        let guid = Guid::parse("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap();
        assert_guid(
            &guid,
            0xffffffff,
            0xffff,
            0xffff,
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        );
    }

    #[test]
    fn parse_too_short() {
        assert!(Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102").is_err());
    }

    #[test]
    fn parse_too_long() {
        assert!(Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4ff").is_err());
    }

    #[test]
    fn parse_invalid_character() {
        assert!(Guid::parse("zaa02ab1-c26e-431b-8efe-85ee8de102e4").is_err());
    }

    #[test]
    fn parse_empty_string() {
        assert!(Guid::parse("").is_err());
    }

    #[test]
    fn parse_works_in_const_context() {
        const GUID: Guid = match Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4") {
            Ok(g) => g,
            Err(_) => panic!("invalid GUID"),
        };
        assert_guid(
            &GUID,
            0x2aa02ab1,
            0xc26e,
            0x431b,
            [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
        );
    }

    #[test]
    fn parse_with_dashes_equals_without() {
        let with = Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").unwrap();
        let without = Guid::parse("2aa02ab1c26e431b8efe85ee8de102e4").unwrap();
        assert_eq!(with, without);
    }

    #[test]
    fn parse_dashes_in_nonstandard_positions() {
        let guid = Guid::parse("2a-a02ab1c2-6e431b8efe85ee8d-e102e4").unwrap();
        assert_guid(
            &guid,
            0x2aa02ab1,
            0xc26e,
            0x431b,
            [0x8e, 0xfe, 0x85, 0xee, 0x8d, 0xe1, 0x02, 0xe4],
        );
    }

    fn assert_guid(guid: &Guid, data1: u32, data2: u16, data3: u16, data4: [u8; 8]) {
        let raw = guid.to_raw();
        assert_eq!(raw.Data1, data1, "Data1 mismatch");
        assert_eq!(raw.Data2, data2, "Data2 mismatch");
        assert_eq!(raw.Data3, data3, "Data3 mismatch");
        assert_eq!(raw.Data4, data4, "Data4 mismatch");
    }
}
