mod can_client;
mod elf_image;

use can_client::{CanClient, ValidationResult};
use clap::{Parser, Subcommand};
use elf_image::FirmwareImage;

#[derive(Parser)]
#[command(name = "eoi-flash-tool", about = "Flash firmware to EoI boards via CAN bus")]
struct Cli {
    /// CAN interface to use
    #[arg(short, long, default_value = "can0")]
    interface: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let client = CanClient::connect(&cli.interface).await?;

    match cli.command {
        Command::State => {
            let state = client.get_state().await?;
            log::info!("Device state: {}", state);
        }
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
        Command::Flash { elf_file, no_start } => {
            flash(&client, &elf_file, no_start).await?;
        }
    }

    Ok(())
}

async fn flash(
    client: &CanClient,
    elf_file: &str,
    no_start: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse ELF file
    log::info!("Parsing ELF file: {}", elf_file);
    let firmware = FirmwareImage::from_elf_file(elf_file)?;
    log::info!(
        "Firmware: {} bytes app + 2048 bytes header = {} bytes total",
        firmware.app_size,
        firmware.data.len()
    );

    // If app is running (no bootloader response), reboot into bootloader first
    log::info!("Reading device state...");
    match client.get_state().await {
        Ok(state) => log::info!("Device state: {}", state),
        Err(_) => {
            log::info!("No bootloader response, sending reboot command...");
            // Send reboot — the app will reset, bootloader starts and waits 2s
            let _ = client.reboot().await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let state = client.get_state().await?;
            log::info!("Device state after reboot: {}", state);
        }
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
