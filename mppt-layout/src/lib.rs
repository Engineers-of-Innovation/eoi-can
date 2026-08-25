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

/// The ID strap a GaN MPPT at `position` on `side` carries -- the inverse of
/// [`gan_side_and_position`], so `LAYOUT` can be written in the F/R names the boat
/// is labelled with instead of raw strap numbers.
pub const fn gan_strap(side: Side, position: u8) -> u8 {
    let side_bit = match side {
        Side::Front => 0b1000,
        Side::Rear => 0,
    };
    side_bit | (position & 0b111)
}

/// A GaN MPPT named the way the boat labels it: `gan(Side::Front, 1)` is F1.
pub const fn gan(side: Side, position: u8) -> MpptKind {
    MpptKind::Gan {
        node: gan_strap(side, position),
    }
}

/// Order = physical position on the boat. Index 0 = bow-most (position 1),
/// last element = stern-most. Edit this list to re-map MPPTs to positions.
///
/// The boat sails on GaN MPPTs alone now: three forward -- F1, F4, F7 -- then eight
/// aft, R0-R7. Addressing is rule-based, so the list is too: the forward block
/// comes first and strap positions ascend within each side, which
/// `layout_follows_addressing_rule` holds this list to. Legacy controllers are no
/// longer fitted, but `MpptKind::Legacy` entries can be mixed back in here if one
/// returns -- nothing else in this crate assumes the layout is all GaN.
pub const LAYOUT: &[MpptKind] = &[
    gan(Side::Front, 1),
    gan(Side::Front, 4),
    gan(Side::Front, 7),
    gan(Side::Rear, 0),
    gan(Side::Rear, 1),
    gan(Side::Rear, 2),
    gan(Side::Rear, 3),
    gan(Side::Rear, 4),
    gan(Side::Rear, 5),
    gan(Side::Rear, 6),
    gan(Side::Rear, 7),
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
    fn gan_strap_round_trips() {
        for side in [Side::Front, Side::Rear] {
            for position in 0..8 {
                assert_eq!(
                    gan_side_and_position(gan_strap(side, position)),
                    (side, position)
                );
            }
        }
    }

    /// The boat's GaN addressing is rule-based: everything forward comes before
    /// everything aft, and strap positions ascend within a side. A hand edit that
    /// breaks that silently renumbers panels, so hold the list to the rule.
    #[test]
    fn layout_follows_addressing_rule() {
        let mut seen_rear = false;
        let mut last: Option<(Side, u8)> = None;
        for kind in LAYOUT {
            let MpptKind::Gan { node } = *kind else {
                continue;
            };
            let (side, position) = gan_side_and_position(node);
            if side == Side::Rear {
                seen_rear = true;
            } else {
                assert!(!seen_rear, "F{position} sits aft of a rear MPPT in LAYOUT");
            }
            if let Some((last_side, last_position)) = last {
                if last_side == side {
                    assert!(
                        position > last_position,
                        "strap positions must ascend within a side: {side:?} {last_position} then {position}",
                    );
                }
            }
            last = Some((side, position));
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
        assert_eq!(position_of(gan(Side::Front, 0)), None);
    }
}
