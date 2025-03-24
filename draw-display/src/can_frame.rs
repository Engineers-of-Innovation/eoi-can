use core::fmt::{Debug, Formatter};

/// CAN frame consisting of ID and data
#[derive(Clone, PartialEq, Eq)]
pub struct CanFrame {
    /// The ID of the frame
    pub id: embedded_can::Id,
    /// The payload of the frame
    pub data: heapless::Vec<u8, { Self::MAX_LEN }>,
}

impl CanFrame {
    const MAX_LEN: usize = 8;

    /// Wrap an already encoded CAN frame
    pub fn from_encoded<const LEN: usize>(id: embedded_can::Id, data: &[u8; LEN]) -> Self {
        const {
            assert!(LEN <= Self::MAX_LEN);
        }

        Self {
            id,
            data: data.as_slice().try_into().expect("checked by const assert"),
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
