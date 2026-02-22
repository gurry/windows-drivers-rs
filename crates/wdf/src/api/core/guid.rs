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
    /// This is a `const fn`, so it can be used in const contexts (e.g.
    /// `const` items and `static` initializers).
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
        let hex_digits = match Self::extract_hex_digits(guid_str.as_bytes()) {
            Ok(buf) => buf,
            Err(e) => return Err(e),
        };

        let data1 = Self::parse_hex_u32(&hex_digits, 0, 8);
        let data2 = Self::parse_hex_u32(&hex_digits, 8, 4) as u16;
        let data3 = Self::parse_hex_u32(&hex_digits, 12, 4) as u16;
        let data4 = Self::parse_hex_u8_array(&hex_digits, 16);

        Ok(Guid(GUID {
            Data1: data1,
            Data2: data2,
            Data3: data3,
            Data4: data4,
        }))
    }

    /// Extracts hex digits from `bytes`, skipping any dashes.
    /// Returns them collected into a fixed-size buffer.
    const fn extract_hex_digits(bytes: &[u8]) -> Result<[u8; 32], &'static str> {
        const BUF_SIZE: usize = 32;
        const ERR: &str = "Invalid GUID format: expected 32 hex digits, with optional dashes";

        let mut hex_digits = [0u8; BUF_SIZE];
        let mut filled = 0;
        let mut i = 0;

        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'-' => {}
                b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                    if filled >= BUF_SIZE {
                        return Err(ERR);
                    }
                    hex_digits[filled] = b;
                    filled += 1;
                }
                _ => return Err(ERR),
            }
            i += 1;
        }

        if filled != BUF_SIZE {
            return Err(ERR);
        }

        Ok(hex_digits)
    }

    /// Parses `count` hex digits starting at `offset` into a `u32`.
    const fn parse_hex_u32(digits: &[u8; 32], offset: usize, count: usize) -> u32 {
        let mut value: u32 = 0;
        let mut i = 0;
        while i < count {
            value = (value << 4) | Self::hex_digit_to_value(digits[offset + i]) as u32;
            i += 1;
        }
        value
    }

    /// Parses 8 consecutive byte pairs starting at `offset` into a `[u8; 8]`.
    const fn parse_hex_u8_array(digits: &[u8; 32], offset: usize) -> [u8; 8] {
        let mut result = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            let hi = Self::hex_digit_to_value(digits[offset + i * 2]);
            let lo = Self::hex_digit_to_value(digits[offset + i * 2 + 1]);
            result[i] = (hi << 4) | lo;
            i += 1;
        }
        result
    }

    /// Converts a single hex ASCII byte to its numeric value (0..15).
    /// Caller must ensure `b` is a valid hex digit.
    const fn hex_digit_to_value(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
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
