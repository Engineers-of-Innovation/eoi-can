use core::fmt::{Debug, Formatter};

/// CAN frame consisting of ID and data
#[derive(Clone, PartialEq, Eq)]
pub struct CanFrame {
    /// The ID of the frame
    pub id: embedded_can::Id,
    /// The payload of the frame
    pub data: heapless::Vec<u8, { Self::MAX_LEN }>,
}

#[cfg(feature = "arbitrary")]
use embedded_can::{ExtendedId, StandardId};
#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for CanFrame {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let id: u32 = u.int_in_range(0..=0x1FFFFFFF)?;
        let id = if id <= 0x7FF {
            embedded_can::Id::Standard(StandardId::new(id as u16).unwrap())
        } else {
            embedded_can::Id::Extended(ExtendedId::new(id).unwrap())
        };
        let data_len = u.int_in_range(0..=Self::MAX_LEN as u8)?;
        let mut data = heapless::Vec::new();
        for _ in 0..data_len {
            data.push(u.int_in_range(0..=255)?)
                .expect("Data length exceeds MAX_LEN");
        }
        Ok(Self { id, data })
    }
}

impl CanFrame {
    const MAX_LEN: usize = 8;

    /// Wrap an already encoded CAN frame
    pub fn from_encoded(id: embedded_can::Id, data: &[u8]) -> Self {
        Self {
            id,
            data: heapless::Vec::from_slice(data).expect("Data length exceeds MAX_LEN"),
        }
    }
}

impl Debug for CanFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        struct DebugId(embedded_can::Id);
        impl Debug for DebugId {
            fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    embedded_can::Id::Extended(e) => {
                        f.write_fmt(format_args!("{:#010X}", e.as_raw()))
                    }
                    embedded_can::Id::Standard(s) => {
                        f.write_fmt(format_args!("{:#06X}", s.as_raw()))
                    }
                }
            }
        }

        struct DebugData<'a>(&'a [u8]);
        impl Debug for DebugData<'_> {
            fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
                let mut list = f.debug_list();
                for &elem in self.0 {
                    list.entry(&format_args!("{:#04X}", elem));
                }
                list.finish()
            }
        }

        f.debug_struct("CanFrame")
            .field("id", &DebugId(self.id))
            .field("data", &DebugData(&self.data))
            .finish()
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for CanFrame {
    fn format(&self, fmt: defmt::Formatter) {
        struct DebugId(embedded_can::Id);
        impl defmt::Format for DebugId {
            fn format(&self, fmt: defmt::Formatter) {
                match self.0 {
                    embedded_can::Id::Extended(e) => defmt::write!(fmt, "{=u32:#010X}", e.as_raw()),
                    embedded_can::Id::Standard(s) => defmt::write!(fmt, "{=u16:#06X}", s.as_raw()),
                }
            }
        }

        defmt::write!(
            fmt,
            "CanFrame {{ id: {}, data: {=[u8]:#04X} }}",
            &DebugId(self.id),
            &self.data
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_can::{ExtendedId, StandardId};
    use std::format;

    fn ext(id: u32, data: &[u8]) -> CanFrame {
        CanFrame {
            id: ExtendedId::new(id).unwrap().into(),
            data: data.iter().copied().collect(),
        }
    }

    fn std(id: u16, data: &[u8]) -> CanFrame {
        CanFrame {
            id: StandardId::new(id).unwrap().into(),
            data: data.iter().copied().collect(),
        }
    }

    #[test]
    fn debug_formats_nicely() {
        let debug = format!("{:?}", ext(0x2A, &[]));
        assert_eq!(debug, "CanFrame { id: 0x0000002A, data: [] }");

        let debug = format!("{:?}", std(0x2A, &[]));
        assert_eq!(debug, "CanFrame { id: 0x002A, data: [] }");

        let debug = format!("{:?}", std(0x2A, &[1, 2, 3, 4, 15, 16, 255]));
        assert_eq!(
            debug,
            "CanFrame { id: 0x002A, data: [0x01, 0x02, 0x03, 0x04, 0x0F, 0x10, 0xFF] }"
        );
    }
}
