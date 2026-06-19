use crate::domain::remote_access::{DetectedInterface, NetworkInterfaceGateway, VpnInterface};

pub fn get_network_info(gateway: &dyn NetworkInterfaceGateway) -> Vec<DetectedInterface> {
    gateway.detect_all_interfaces()
}

pub fn detect_vpn_tunnel(gateway: &dyn NetworkInterfaceGateway) -> Option<VpnInterface> {
    gateway.detect_vpn_ip()
}
