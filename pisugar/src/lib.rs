use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn battery_info() -> Result<(f32, bool), Box<dyn std::error::Error + Send + Sync>> {
    let addr = "127.0.0.1:8423";

    let command_soc = b"get battery\n";
    let command_charging = b"get battery_power_plugged\n";

    let soc = {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(command_soc).await?;
        stream.shutdown().await?;
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;
        let buffer_str = String::from_utf8_lossy(&buffer);
        let soc_str = buffer_str
            .split(':')
            .nth(1)
            .ok_or("Failed to parse SOC from response")?;
        soc_str.trim().parse()?
    };

    let charging = {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(command_charging).await?;
        stream.shutdown().await?;
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;
        let buffer_str = String::from_utf8_lossy(&buffer);
        let charging_str = buffer_str
            .split(':')
            .nth(1)
            .ok_or("Failed to parse charging status from response")?;
        charging_str.trim() == "true"
    };

    Ok((soc, charging))
}
