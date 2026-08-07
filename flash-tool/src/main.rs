mod can_client;
mod elf_image;

use can_client::{CanClient, ValidationResult, format_version};
use clap::{Parser, Subcommand, ValueEnum};
use elf_image::FirmwareImage;
use eoi_boot_api::header::AppType;
use eoi_boot_api::protocol::board_address;

#[derive(Parser)]
#[command(name = "eoi-flash-tool", about = "Flash firmware to EoI boards via CAN bus")]
struct Cli {
    /// CAN interface to use
    #[arg(short, long, default_value = "can0")]
    interface: String,

    /// Board to address. Required for every command except `scan`; for `flash`
    /// it defaults to the board the ELF was built for.
    #[arg(short, long)]
    board: Option<Board>,

    #[command(subcommand)]
    command: Command,
}

/// The boards on the bus. Names match the firmware binary names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Board {
    RudderController,
    HeightSensorController,
    Dashboard,
}

impl From<Board> for AppType {
    fn from(b: Board) -> Self {
        match b {
            Board::RudderController => AppType::RudderController,
            Board::HeightSensorController => AppType::HeightSensorController,
            Board::Dashboard => AppType::Dashboard,
        }
    }
}

/// Name of the firmware binary for an app type, for human-facing output.
fn board_name(t: AppType) -> &'static str {
    match t {
        AppType::RudderController => "rudder-controller",
        AppType::HeightSensorController => "height-sensor-controller",
        AppType::Dashboard => "dashboard",
    }
}

#[derive(Subcommand)]
enum Command {
    /// List every board answering on the bus, with its bootloader state
    Scan,
    /// Flash an ELF firmware file (erase + write + validate + boot)
    Flash {
        /// Path to the ELF firmware file
        elf_file: String,
        /// Do not boot the application after flashing
        #[arg(long)]
        no_start: bool,
    },
    /// Erase the application partition
    Erase,
    /// Boot the application
    Boot,
    /// Reboot the device into bootloader
    Reboot,
    /// Read the current device state
    State,
    /// Read the version and git hash of whatever is running on the device
    Version,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(e) = run().await {
        // Use Display, not Debug, so thiserror's #[error(...)] messages reach the user.
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // `scan` is the only command that is not addressed to one board.
    if let Command::Scan = cli.command {
        return scan(&cli.interface).await;
    }

    // `flash` takes its target from the ELF; everything else must be told.
    let (target, firmware): (AppType, Option<FirmwareImage>) = match (&cli.command, cli.board) {
        (Command::Flash { elf_file, .. }, board) => {
            log::info!("Parsing ELF file: {}", elf_file);
            let firmware = FirmwareImage::from_elf_file(elf_file)?;
            if let Some(board) = board {
                let requested: AppType = board.into();
                if requested != firmware.app_type {
                    return Err(format!(
                        "--board {} does not match the ELF, which was built for {}",
                        board_name(requested),
                        board_name(firmware.app_type)
                    )
                    .into());
                }
            }
            (firmware.app_type, Some(firmware))
        }
        (_, Some(board)) => (board.into(), None),
        (_, None) => {
            return Err("this command needs --board <BOARD> to say which board to address; \
                        run `scan` to see what is on the bus"
                .into());
        }
    };

    let addr = board_address(target);
    log::info!(
        "Target: {} (cmd 0x{:03X}, resp 0x{:03X}, data 0x{:03X})",
        board_name(target),
        addr.cmd,
        addr.resp,
        addr.data
    );
    let client = CanClient::connect(&cli.interface, target).await?;

    match cli.command {
        Command::Scan => unreachable!("handled above"),
        Command::State => {
            let device = client.get_state().await?;
            log::info!(
                "Device state: {} (app type {})",
                device.state,
                device.app_type.map(board_name).unwrap_or("not reported")
            );
        }
        Command::Version => match client.get_version().await? {
            Some(version) => println!("{:<26} {}", board_name(target), format_version(&version)),
            None => {
                println!(
                    "{:<26} answered, but reports no version — firmware predates GetVersion",
                    board_name(target)
                );
            }
        },
        Command::Erase => {
            log::info!("Erasing application...");
            client.erase_app().await?;
            log::info!("Erase complete");
        }
        Command::Boot => {
            log::info!("Booting application...");
            client.boot_app().await?;
            log::info!("Boot command sent");
        }
        Command::Reboot => {
            log::info!("Rebooting device...");
            client.reboot().await?;
            log::info!("Reboot command sent");
        }
        Command::Flash { no_start, .. } => {
            let firmware = firmware.expect("parsed above for the Flash arm");
            flash(&client, firmware, target, no_start).await?;
        }
    }

    Ok(())
}

async fn scan(interface: &str) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Scanning {interface} for boards...");
    let found = CanClient::discover(interface).await?;
    if found.is_empty() {
        println!("Nothing answered. Check the interface, the bitrate and the bus wiring.");
        return Ok(());
    }
    println!("{:<26} {:<18} VERSION", "BOARD", "STATE");
    for d in &found {
        let name = d.app_type.map(board_name).unwrap_or("unknown");
        let version = d
            .version
            .as_ref()
            .map(format_version)
            .unwrap_or_else(|| "not reported".to_string());
        // `d.state` through `{:<18}` directly would ignore the width — the
        // Display impl does not call `f.pad`.
        println!("{:<26} {:<18} {}", name, d.state.to_string(), version);
    }
    if found.iter().any(|d| !d.state.in_bootloader()) {
        println!();
        println!("Boards in AppRunning are executing their application. Use");
        println!("`--board <BOARD> reboot` to bring one into the bootloader.");
    }
    Ok(())
}

async fn flash(
    client: &CanClient,
    firmware: FirmwareImage,
    target: AppType,
    no_start: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!(
        "Firmware: app_type={:?}, {} bytes app + 2048 bytes header = {} bytes total",
        firmware.app_type,
        firmware.app_size,
        firmware.data.len()
    );

    // Get into the bootloader first. Two ways the app can be in the way: it
    // answers `AppRunning`, or (firmware predating that reply) it answers
    // nothing at all. Both take the same reboot path — the reboot goes to this
    // board's command ID, so other boards keep running.
    log::info!("Reading device state...");
    let device = match client.get_state().await {
        Ok(device) if device.state.in_bootloader() => {
            log::info!("Device state: {}", device.state);
            device
        }
        outcome => {
            match &outcome {
                Ok(device) => log::info!("Board is running its application ({})", device.state),
                Err(_) => log::info!("No bootloader response, sending reboot command..."),
            }
            // Send reboot — the app will reset, bootloader starts and waits 2s
            let _ = client.reboot().await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let device = client.get_state().await?;
            log::info!("Device state after reboot: {}", device.state);
            if !device.state.in_bootloader() {
                return Err(format!(
                    "Board is still in state {} after a reboot — refusing to erase",
                    device.state
                )
                .into());
            }
            device
        }
    };

    // Confirm identity BEFORE erasing. The erase is the destructive step, so a
    // mismatch has to fail here rather than being caught by validation after the
    // working application is already gone.
    if let Some(reported) = device.app_type
        && reported != target
    {
        return Err(format!(
            "Board answering on {}'s address reports app type {} — refusing to erase. \
             Its bootloader was built for the wrong board.",
            board_name(target),
            board_name(reported)
        )
        .into());
    }

    // Erase
    log::info!("Erasing application...");
    client.erase_app().await?;
    log::info!("Erase complete");

    // Write firmware (header + app)
    log::info!("Writing firmware...");
    client.write_firmware(&firmware.data).await?;
    log::info!("Write complete");

    // Validate
    log::info!("Validating application...");
    let result = client.validate_app().await?;
    if result != ValidationResult::Valid {
        if result == ValidationResult::WrongAppType {
            return Err(format!(
                "Validation failed: device rejected app type {:?} — wrong bootloader variant for this board",
                firmware.app_type
            )
            .into());
        }
        return Err(format!("Validation failed: {}", result).into());
    }
    log::info!("Validation: {}", result);

    // Boot
    if !no_start {
        log::info!("Booting application...");
        client.boot_app().await?;
        log::info!("Boot command sent, device will start in 500ms");
    } else {
        log::info!("Skipping boot (--no-start)");
    }

    log::info!("Flash complete!");
    Ok(())
}
