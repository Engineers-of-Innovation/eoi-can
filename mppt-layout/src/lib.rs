#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpptKind {
    Legacy { node: u8, channel: u8 },
    // `Gan`'s node is the ID strap (0-15), not the CAN node number -- see
    // `gan_side_and_position`.
    Gan { node: u8 },
}

/// Which side of the boat a GaN MPPT sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Rear,
}

/// Number of GaN MPPT ID straps, and so the number of MPPTs the boat can carry:
/// R0-R7 and F0-F7.
pub const GAN_STRAP_COUNT: usize = 16;

/// The side and 0-based position a GaN MPPT's ID strap encodes.
///
/// A GaN MPPT's CAN ID is `(node << 4) | packet` with `node = 64 + strap`. Within
/// the strap, bit 3 selects the side (1 = Front, 0 = Rear) and bits 0-2 the
/// position:
///
/// | MPPT | Node | CAN IDs | | MPPT | Node | CAN IDs |
/// | --- | --- | --- | --- | --- | --- | --- |
/// | R0 | 64 | `0x400`-`0x402` | | F0 | 72 | `0x480`-`0x482` |
/// | R7 | 71 | `0x470`-`0x472` | | F7 | 79 | `0x4F0`-`0x4F2` |
///
/// Within each block: +0 Power, +1 Status, +2 Sweep.
///
/// The baseboards sit outside this scheme -- `0x4E6` Front and `0x4EE` Rear. They
/// share node numbers with the position-6 MPPTs but use packet ids outside 0-2, so
/// they never decode as MPPT data and cannot be confused with one.
pub const fn gan_side_and_position(strap: u8) -> (Side, u8) {
    let side = if strap & 0b1000 != 0 {
        Side::Front
    } else {
        Side::Rear
    };
    (side, strap & 0b111)
}

/// Order = physical position on the boat. Index 0 = bow-most (position 1),
/// last element = stern-most. Edit this list to re-map MPPTs to positions.
pub const LAYOUT: &[MpptKind] = &[
    MpptKind::Legacy {
        node: 5,
        channel: 1,
    },
    MpptKind::Legacy {
        node: 5,
        channel: 2,
    },
    MpptKind::Legacy {
        node: 2,
        channel: 2,
    },
    MpptKind::Legacy {
        node: 2,
        channel: 3,
    },
    MpptKind::Gan { node: 0 },
    MpptKind::Gan { node: 1 },
    MpptKind::Gan { node: 2 },
    MpptKind::Gan { node: 3 },
    MpptKind::Gan { node: 4 },
    MpptKind::Gan { node: 6 },
    MpptKind::Gan { node: 7 },
];

/// 1-based boat position for the given MPPT identity, or `None` if unmapped.
pub const fn position_of(kind: MpptKind) -> Option<u8> {
    let mut i = 0;
    while i < LAYOUT.len() {
        if kind_eq(LAYOUT[i], kind) {
            return Some((i + 1) as u8);
        }
        i += 1;
    }
    None
}

const fn kind_eq(a: MpptKind, b: MpptKind) -> bool {
    match (a, b) {
        (
            MpptKind::Legacy {
                node: an,
                channel: ac,
            },
            MpptKind::Legacy {
                node: bn,
                channel: bc,
            },
        ) => an == bn && ac == bc,
        (MpptKind::Gan { node: an }, MpptKind::Gan { node: bn }) => an == bn,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_entries() {
        for (i, a) in LAYOUT.iter().enumerate() {
            for (j, b) in LAYOUT.iter().enumerate().skip(i + 1) {
                assert!(
                    !kind_eq(*a, *b),
                    "duplicate LAYOUT entry at indices {i} and {j}: {a:?}",
                );
            }
        }
    }

    #[test]
    fn position_round_trip() {
        for (i, kind) in LAYOUT.iter().enumerate() {
            assert_eq!(position_of(*kind), Some((i + 1) as u8));
        }
    }

    #[test]
    fn unmapped_returns_none() {
        assert_eq!(
            position_of(MpptKind::Legacy {
                node: 7,
                channel: 0
            }),
            None
        );
        assert_eq!(position_of(MpptKind::Gan { node: 15 }), None);
    }
}
