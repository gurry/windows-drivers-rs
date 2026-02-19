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

    pub fn parse(guid_str: &str) -> Result<Self, &'static str> {
        // Remove dashes from the input string
        let guid_str = guid_str.replace("-", "");

        let err = "Invalid GUID format";

        if guid_str.len() != 32 {
            return Err(err);
        }

        let data1 = u32::from_str_radix(&guid_str[0..8], 16).map_err(|_| err)?;
        let data2 = u16::from_str_radix(&guid_str[8..12], 16).map_err(|_| err)?;
        let data3 = u16::from_str_radix(&guid_str[12..16], 16).map_err(|_| err)?;

        let mut data4 = [0u8; 8];
        for i in 0..8 {
            data4[i] =
                u8::from_str_radix(&guid_str[16 + i * 2..18 + i * 2], 16).map_err(|_| err)?;
        }

        Ok(Guid(GUID {
            Data1: data1,
            Data2: data2,
            Data3: data3,
            Data4: data4,
        }))
    }

    /// Returns the underlying raw `GUID` by value.
    pub fn to_raw(&self) -> GUID {
        self.0
    }

    pub fn as_lpcguid(&self) -> *const GUID {
        &self.0
    }
}
