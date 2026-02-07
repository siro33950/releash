use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;

const CGNAT_BASE: u32 = 0x6440_0000; // 100.64.0.0
const CGNAT_MASK: u32 = 0xFFC0_0000; // /10

const VPN_INTERFACE_PREFIXES: &[&str] = &["nordlynx", "tailscale", "utun", "wg", "tun"];

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let bits: u32 = ip.into();
    (bits & CGNAT_MASK) == CGNAT_BASE
}

fn is_vpn_interface(name: &str) -> bool {
    let lower = name.to_lowercase();
    VPN_INTERFACE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

#[derive(Debug, Clone)]
pub struct VpnInterface {
    pub name: String,
    pub ip: Ipv4Addr,
}

fn is_private_ip(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    match octets[0] {
        10 => true,
        172 => (16..=31).contains(&octets[1]),
        192 => octets[1] == 168,
        _ => false,
    }
}

pub fn detect_local_ip() -> Option<Ipv4Addr> {
    let interfaces = list_network_interfaces().ok()?;
    interfaces
        .into_iter()
        .find(|iface| {
            !iface.ip.is_loopback() && !is_vpn_interface(&iface.name) && is_private_ip(iface.ip)
        })
        .map(|iface| iface.ip)
}

pub fn detect_vpn_ip() -> Option<VpnInterface> {
    let interfaces = list_network_interfaces().ok()?;

    interfaces
        .into_iter()
        .find(|iface| is_vpn_interface(&iface.name) && is_cgnat(iface.ip))
        .map(|iface| VpnInterface {
            name: iface.name,
            ip: iface.ip,
        })
}

#[derive(Debug)]
struct RawInterface {
    name: String,
    ip: Ipv4Addr,
}

fn list_network_interfaces() -> Result<Vec<RawInterface>, String> {
    let output = Command::new("ifconfig")
        .output()
        .map_err(|e| format!("ifconfig 実行失敗: {e}"))?;

    let stdout = String::from_utf8(output.stdout).map_err(|e| format!("UTF-8 パース失敗: {e}"))?;

    parse_ifconfig_output(&stdout)
}

fn parse_ifconfig_output(output: &str) -> Result<Vec<RawInterface>, String> {
    let mut interfaces = Vec::new();
    let mut current_name: Option<String> = None;

    for line in output.lines() {
        if !line.starts_with('\t') && !line.starts_with(' ') && line.contains(": flags=") {
            current_name = line.split(':').next().map(|s| s.to_string());
        } else if let Some(ref name) = current_name {
            let trimmed = line.trim();
            if trimmed.starts_with("inet ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(ip) = parts[1].parse::<Ipv4Addr>() {
                        interfaces.push(RawInterface {
                            name: name.clone(),
                            ip,
                        });
                    }
                }
            }
        }
    }

    Ok(interfaces)
}

#[allow(dead_code)]
pub fn resolve_bind_address(bind: &str) -> Result<IpAddr, String> {
    match bind {
        "meshnet" => {
            if let Some(iface) = detect_vpn_ip() {
                log::info!("VPNトンネル検出: {} ({})", iface.name, iface.ip);
                Ok(IpAddr::V4(iface.ip))
            } else {
                log::warn!("VPNトンネルが見つかりません。手動でIPアドレスを指定してください。");
                Err("VPNトンネルが見つかりません".to_string())
            }
        }
        "any" => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        addr => addr
            .parse::<IpAddr>()
            .map_err(|e| format!("無効なIPアドレス: {e}")),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NetworkInfo {
    pub meshnet: Option<String>,
    pub lan: Option<String>,
}

#[tauri::command]
pub fn get_network_info() -> NetworkInfo {
    NetworkInfo {
        meshnet: detect_vpn_ip().map(|iface| iface.ip.to_string()),
        lan: detect_local_ip().map(|ip| ip.to_string()),
    }
}

#[tauri::command]
pub fn detect_vpn_tunnel() -> Result<Option<serde_json::Value>, String> {
    Ok(detect_vpn_ip().map(|iface| {
        serde_json::json!({
            "name": iface.name,
            "ip": iface.ip.to_string()
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cgnat_in_range() {
        assert!(is_cgnat(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_cgnat(Ipv4Addr::new(100, 100, 1, 1)));
        assert!(is_cgnat(Ipv4Addr::new(100, 127, 255, 254)));
    }

    #[test]
    fn test_cgnat_out_of_range() {
        assert!(!is_cgnat(Ipv4Addr::new(100, 63, 255, 255)));
        assert!(!is_cgnat(Ipv4Addr::new(100, 128, 0, 0)));
        assert!(!is_cgnat(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(!is_cgnat(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_vpn_interface_names() {
        assert!(is_vpn_interface("nordlynx"));
        assert!(is_vpn_interface("tailscale0"));
        assert!(is_vpn_interface("utun3"));
        assert!(is_vpn_interface("wg0"));
        assert!(is_vpn_interface("tun0"));
    }

    #[test]
    fn test_non_vpn_interface_names() {
        assert!(!is_vpn_interface("en0"));
        assert!(!is_vpn_interface("lo0"));
        assert!(!is_vpn_interface("eth0"));
        assert!(!is_vpn_interface("bridge0"));
    }

    #[test]
    fn test_parse_ifconfig_with_vpn() {
        let output = r#"en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	inet 192.168.1.100 netmask 0xffffff00 broadcast 192.168.1.255
utun3: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1280
	inet 100.100.1.42 --> 100.100.1.42 netmask 0xffffffff
lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
	inet 127.0.0.1 netmask 0xff000000
"#;
        let interfaces = parse_ifconfig_output(output).unwrap();

        assert_eq!(interfaces.len(), 3);
        assert_eq!(interfaces[0].name, "en0");
        assert_eq!(interfaces[0].ip, Ipv4Addr::new(192, 168, 1, 100));
        assert_eq!(interfaces[1].name, "utun3");
        assert_eq!(interfaces[1].ip, Ipv4Addr::new(100, 100, 1, 42));
        assert_eq!(interfaces[2].name, "lo0");
        assert_eq!(interfaces[2].ip, Ipv4Addr::new(127, 0, 0, 1));
    }

    #[test]
    fn test_parse_ifconfig_empty() {
        let interfaces = parse_ifconfig_output("").unwrap();
        assert!(interfaces.is_empty());
    }

    #[test]
    fn test_resolve_bind_address_localhost() {
        let result = resolve_bind_address("127.0.0.1").unwrap();
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn test_resolve_bind_address_any() {
        let result = resolve_bind_address("any").unwrap();
        assert_eq!(result, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn test_resolve_bind_address_invalid() {
        let result = resolve_bind_address("not_an_ip");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip(Ipv4Addr::new(192, 168, 1, 100)));
        assert!(is_private_ip(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ip(Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_private_ip(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(!is_private_ip(Ipv4Addr::new(172, 15, 0, 1)));
        assert!(!is_private_ip(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(!is_private_ip(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_private_ip(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn test_cgnat_boundary_values() {
        // 100.64.0.0 = first address in range
        assert!(is_cgnat(Ipv4Addr::new(100, 64, 0, 0)));
        // 100.127.255.255 = last address in range
        assert!(is_cgnat(Ipv4Addr::new(100, 127, 255, 255)));
    }
}
