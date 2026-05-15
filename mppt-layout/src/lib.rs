#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpptKind {
    Legacy { node: u8, channel: u8 },
    Gan { node: u8 },
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
        for i in 0..LAYOUT.len() {
            for j in (i + 1)..LAYOUT.len() {
                assert!(
                    !kind_eq(LAYOUT[i], LAYOUT[j]),
                    "duplicate LAYOUT entry at indices {i} and {j}: {:?}",
                    LAYOUT[i],
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
