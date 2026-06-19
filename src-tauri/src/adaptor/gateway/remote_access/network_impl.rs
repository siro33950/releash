use std::process::Command;

use crate::domain::remote_access::services::{
    classify_interfaces, parse_ifconfig_output, parse_routes_for_interface, select_vpn_interface,
};
use crate::domain::remote_access::value_objects::RawInterface;
use crate::domain::remote_access::{DetectedInterface, NetworkInterfaceGateway, VpnInterface};

pub struct SystemNetworkInterfaceGateway;

fn has_active_routes(iface_name: &str, own_ip: &str) -> bool {
    let output = Command::new("netstat")
        .args(["-rn", "-f", "inet"])
        .output()
        .ok();

    let Some(output) = output else {
        return false;
    };
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return false;
    };

    parse_routes_for_interface(&stdout, iface_name, own_ip)
}

fn list_network_interfaces() -> Result<Vec<RawInterface>, String> {
    let output = Command::new("ifconfig")
        .output()
        .map_err(|e| format!("ifconfig 実行失敗: {e}"))?;

    let stdout = String::from_utf8(output.stdout).map_err(|e| format!("UTF-8 パース失敗: {e}"))?;

    parse_ifconfig_output(&stdout)
}

impl NetworkInterfaceGateway for SystemNetworkInterfaceGateway {
    fn detect_vpn_ip(&self) -> Option<VpnInterface> {
        let interfaces = list_network_interfaces().ok()?;
        let selected = select_vpn_interface(interfaces, has_active_routes);
        if selected.is_none() {
            log::info!("VPNインターフェースは検出されませんでした");
        }
        selected
    }

    fn detect_all_interfaces(&self) -> Vec<DetectedInterface> {
        let interfaces = match list_network_interfaces() {
            Ok(ifaces) => ifaces,
            Err(_) => return Vec::new(),
        };
        classify_interfaces(&interfaces, has_active_routes)
    }
}

pub fn detect_all_interfaces() -> Vec<DetectedInterface> {
    SystemNetworkInterfaceGateway.detect_all_interfaces()
}
