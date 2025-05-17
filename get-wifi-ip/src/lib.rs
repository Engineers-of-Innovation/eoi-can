use network_interface::NetworkInterface;
use network_interface::NetworkInterfaceConfig;

pub fn get_wifi_ip() -> Option<core::net::Ipv4Addr> {
    let network_interfaces = NetworkInterface::show().unwrap_or(vec![]);
    for itf in network_interfaces.iter() {
        if itf.name.starts_with("w") {
            if let Some(&network_interface::Addr::V4(ip)) = itf.addr.first() {
                return Some(ip.ip);
            }
        }
    }
    None
}
