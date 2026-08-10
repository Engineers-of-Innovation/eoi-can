//! The bootloader ID allocation is the one place a silent off-by-one would
//! re-create the broadcast-erase bug, so it is pinned down here.

use eoi_boot_api::header::AppType;
use eoi_boot_api::protocol::{
    ADDRESS_LAST, AppAction, BoardAddress, CAN_ID_DISCOVERY, STATE_APP_RUNNING, VersionInfo,
    app_action, app_type_from_resp_id, board_address, err, msg,
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

#[test]
fn app_running_state_does_not_collide_with_a_bootloader_state() {
    // The bootloader owns 0..=2 (WaitingWithoutApp / WaitingWithApp /
    // FlashingApp). Reusing one of those would make a running app read as a
    // bootloader and let `flash` erase without rebooting first.
    assert!(STATE_APP_RUNNING > 2);
}

/// Two responders share the `GET_VERSION` frame, so encode and decode have to
/// agree on every flag independently — a swapped bit would mislabel which one
/// answered, or claim a clean build for a dirty one.
#[test]
fn version_round_trips_through_every_flag_combination() {
    for dirty in [false, true] {
        for bootloader in [false, true] {
            for git_unknown in [false, true] {
                let v = VersionInfo {
                    major: 1,
                    minor: 22,
                    patch: 255,
                    git: [0xDE, 0xAD, 0xBE],
                    dirty,
                    bootloader,
                    git_unknown,
                };
                let encoded = v.encode();
                assert_eq!(encoded[0], msg::GET_VERSION);
                assert_eq!(VersionInfo::decode(&encoded), Some(v));
            }
        }
    }
}

#[test]
fn version_decode_rejects_frames_that_are_not_a_version_response() {
    let ok = VersionInfo {
        major: 0,
        minor: 1,
        patch: 0,
        git: [1, 2, 3],
        dirty: false,
        bootloader: true,
        git_unknown: false,
    }
    .encode();
    // Truncated: the state response is three bytes and shares the response ID.
    assert_eq!(VersionInfo::decode(&ok[..7]), None);
    assert_eq!(VersionInfo::decode(&[msg::GET_STATE, 1, 3]), None);
    // Right length, wrong type byte.
    let mut wrong_type = ok;
    wrong_type[0] = msg::GET_STATE;
    assert_eq!(VersionInfo::decode(&wrong_type), None);
}

#[test]
fn from_built_parses_the_strings_the_built_crate_generates() {
    let v = VersionInfo::from_built("0", "12", "3", Some("a3f19c7b2e"), Some(false), false);
    assert_eq!((v.major, v.minor, v.patch), (0, 12, 3));
    assert_eq!(v.git, [0xA3, 0xF1, 0x9C]);
    assert!(!v.dirty);
    assert!(!v.bootloader);
    assert!(!v.git_unknown);

    // Uppercase hash, dirty tree, bootloader.
    let v = VersionInfo::from_built("1", "0", "0", Some("A3F19C"), Some(true), true);
    assert_eq!(v.git, [0xA3, 0xF1, 0x9C]);
    assert!(v.dirty);
    assert!(v.bootloader);

    // Not a git checkout: the hash is meaningless and says so.
    let v = VersionInfo::from_built("1", "0", "0", None, None, false);
    assert!(v.git_unknown);
    assert_eq!(v.git, [0, 0, 0]);
    // Unknown dirtiness must not be reported as clean.
    assert!(v.dirty);

    // A version component past a byte saturates rather than wrapping to a
    // number that looks plausible.
    assert_eq!(
        VersionInfo::from_built("300", "0", "0", None, None, false).major,
        255
    );
    // Cargo allows pre-release suffixes; the numeric prefix is what we report.
    assert_eq!(
        VersionInfo::from_built("2", "1", "0-rc1", None, None, false).patch,
        0
    );
}

/// These all run at compile time inside the firmware, where a panic is a build
/// failure and a wrong answer is a version that identifies nothing. None of the
/// inputs should ever occur, which is exactly why they are worth pinning.
#[test]
fn from_built_survives_degenerate_input_without_panicking() {
    // A hash shorter than six chars fills what it can and zeroes the rest,
    // rather than indexing past the end.
    assert_eq!(
        VersionInfo::from_built("1", "0", "0", Some("a3f1"), Some(false), false).git,
        [0xA3, 0xF1, 0x00]
    );
    assert_eq!(
        VersionInfo::from_built("1", "0", "0", Some(""), Some(false), false).git,
        [0, 0, 0]
    );
    // Odd length: the trailing nibble has no pair, so it is dropped.
    assert_eq!(
        VersionInfo::from_built("1", "0", "0", Some("a3f19"), Some(false), false).git,
        [0xA3, 0xF1, 0x00]
    );
    // Empty and non-numeric version components read as zero, not garbage.
    let v = VersionInfo::from_built("", "x", "0", None, None, false);
    assert_eq!((v.major, v.minor, v.patch), (0, 0, 0));
    // A hash that is Some but unparseable still counts as known — `git_unknown`
    // tracks "was there a git checkout", not "did the hex parse".
    assert!(!VersionInfo::from_built("1", "0", "0", Some("zz"), None, false).git_unknown);
}

// ---------------------------------------------------------------------------
// What a running application answers. These pin down the behaviour that makes
// `scan` / `state` / `version` work against a live app without letting a
// command aimed at one board disturb another.
// ---------------------------------------------------------------------------

const TEST_VERSION: VersionInfo = VersionInfo {
    major: 0,
    minor: 1,
    patch: 0,
    git: [0x7E, 0x21, 0xB0],
    dirty: true,
    bootloader: false,
    git_unknown: false,
};

fn action(frame_id: u16, data: &[u8], app_type: AppType) -> Option<AppAction> {
    app_action(frame_id, data, app_type, &TEST_VERSION)
}

#[test]
fn app_reports_running_state_on_its_command_id_and_on_discovery() {
    for t in ALL {
        let addr = board_address(t);
        let expected = [msg::GET_STATE, STATE_APP_RUNNING, t as u8];
        for id in [addr.cmd, CAN_ID_DISCOVERY] {
            let a = action(id, &[msg::GET_STATE], t).expect("must answer GetState");
            // Always on the board's own response ID, never on the ID the
            // question arrived on — that keeps one transmitter per ID.
            assert_eq!(
                a,
                AppAction::Reply {
                    id: addr.resp,
                    data: {
                        let mut d = [0u8; 8];
                        d[..3].copy_from_slice(&expected);
                        d
                    },
                    len: 3
                }
            );
            assert_eq!(a.payload(), Some(&expected[..]));
        }
    }
}

#[test]
fn app_reports_its_version_on_its_command_id_and_on_discovery() {
    let t = AppType::Dashboard;
    let addr = board_address(t);
    for id in [addr.cmd, CAN_ID_DISCOVERY] {
        let a = action(id, &[msg::GET_VERSION], t).expect("must answer GetVersion");
        assert_eq!(a.payload(), Some(&TEST_VERSION.encode()[..]));
        assert_eq!(
            VersionInfo::decode(a.payload().unwrap()),
            Some(TEST_VERSION)
        );
        match a {
            AppAction::Reply { id, .. } => assert_eq!(id, addr.resp),
            AppAction::Reboot => panic!("GetVersion must not reboot"),
        }
    }
}

/// The whole point of deriving the command ID from the app type: an OTA update
/// of one board must not reset the others, which all run an accept-all filter.
#[test]
fn reboot_only_fires_on_this_boards_command_id() {
    for t in ALL {
        assert_eq!(
            action(board_address(t).cmd, &[msg::REBOOT], t),
            Some(AppAction::Reboot)
        );
        // Another board's command ID, and the broadcast, must never reset us.
        for other in ALL.iter().filter(|o| **o != t) {
            assert_eq!(action(board_address(*other).cmd, &[msg::REBOOT], t), None);
        }
        assert_eq!(action(CAN_ID_DISCOVERY, &[msg::REBOOT], t), None);
    }
}

#[test]
fn commands_that_need_the_bootloader_are_rejected_when_addressed() {
    let t = AppType::RudderController;
    let addr = board_address(t);
    for cmd in [msg::ERASE_APP, msg::VALIDATE_APP, msg::BOOT_APP, 0x7F] {
        let a = action(addr.cmd, &[cmd], t).expect("addressed commands get an answer");
        assert_eq!(a.payload(), Some(&[msg::ERROR, err::APP_RUNNING][..]));
        // An unrecognised broadcast is not ours to reject — another board may
        // own it, and an error on a shared ID would be noise.
        assert_eq!(action(CAN_ID_DISCOVERY, &[cmd], t), None);
    }
}

#[test]
fn frames_that_are_not_ours_or_carry_nothing_are_ignored() {
    let t = AppType::Dashboard;
    // Our own response and data IDs: we transmit the first, the host the second.
    let addr = board_address(t);
    assert_eq!(action(addr.resp, &[msg::GET_STATE], t), None);
    assert_eq!(action(addr.data, &[msg::GET_STATE], t), None);
    // Ordinary application traffic.
    assert_eq!(action(0x210, &[msg::GET_STATE], t), None);
    // A remote/empty frame has no command byte.
    assert_eq!(action(addr.cmd, &[], t), None);
    assert_eq!(action(CAN_ID_DISCOVERY, &[], t), None);
}
