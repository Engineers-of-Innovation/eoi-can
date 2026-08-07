//! The bootloader ID allocation is the one place a silent off-by-one would
//! re-create the broadcast-erase bug, so it is pinned down here.

use eoi_boot_api::header::AppType;
use eoi_boot_api::protocol::{
    ADDRESS_LAST, BoardAddress, CAN_ID_DISCOVERY, app_type_from_resp_id, board_address,
};

const ALL: [AppType; 3] = [
    AppType::RudderController,
    AppType::HeightSensorController,
    AppType::Dashboard,
];

#[test]
fn addresses_match_the_documented_table() {
    assert_eq!(
        board_address(AppType::RudderController),
        BoardAddress {
            cmd: 0x031,
            resp: 0x032,
            data: 0x033
        }
    );
    assert_eq!(
        board_address(AppType::HeightSensorController),
        BoardAddress {
            cmd: 0x034,
            resp: 0x035,
            data: 0x036
        }
    );
    assert_eq!(
        board_address(AppType::Dashboard),
        BoardAddress {
            cmd: 0x037,
            resp: 0x038,
            data: 0x039
        }
    );
}

#[test]
fn no_two_boards_share_an_id_and_none_collide_with_discovery() {
    let mut seen = vec![CAN_ID_DISCOVERY];
    for t in ALL {
        let a = board_address(t);
        for id in [a.cmd, a.resp, a.data] {
            assert!(!seen.contains(&id), "duplicate CAN ID 0x{id:03X}");
            seen.push(id);
        }
    }
}

#[test]
fn all_ids_stay_inside_the_bootloader_block() {
    for t in ALL {
        let a = board_address(t);
        for id in [a.cmd, a.resp, a.data] {
            assert!(
                (CAN_ID_DISCOVERY..=ADDRESS_LAST).contains(&id),
                "0x{id:03X} escapes the 0x030-0x03F block"
            );
        }
    }
}

#[test]
fn resp_ids_map_back_to_their_app_type() {
    for t in ALL {
        assert_eq!(app_type_from_resp_id(board_address(t).resp), Some(t));
        // cmd and data IDs are host-transmitted; they must never resolve to a board.
        assert_eq!(app_type_from_resp_id(board_address(t).cmd), None);
        assert_eq!(app_type_from_resp_id(board_address(t).data), None);
    }
    assert_eq!(app_type_from_resp_id(CAN_ID_DISCOVERY), None);
    assert_eq!(app_type_from_resp_id(0x040), None);
}
