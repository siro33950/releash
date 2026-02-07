use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;

const VPN_INTERFACE_PREFIXES: &[&str] =
    &["nordlynx", "tailscale", "utun", "wg", "tun", "zt", "nebula"];

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

pub fn detect_vpn_ip() -> Option<VpnInterface> {
    let interfaces = list_network_interfaces().ok()?;

    let candidate = interfaces
        .into_iter()
        .find(|iface| is_vpn_interface(&iface.name))?;

    if !has_active_routes(&candidate.name, &candidate.ip.to_string()) {
        log::info!(
            "VPNインターフェース {} は検出されましたがルートが無効です",
            candidate.name
        );
        return None;
    }

    Some(VpnInterface {
        name: candidate.name,
        ip: candidate.ip,
    })
}

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

fn parse_routes_for_interface(netstat_output: &str, iface_name: &str, own_ip: &str) -> bool {
    for line in netstat_output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // macOS netstat -rn: Destination  Gateway  Flags  Netif  Expire
        if fields.len() < 4 || fields[3] != iface_name {
            continue;
        }
        let dest = fields[0];
        // 自身のIPホストルート、マルチキャスト、ブロードキャストは除外
        if dest == own_ip || dest == format!("{own_ip}/32") {
            continue;
        }
        if dest.starts_with("224.") || dest.starts_with("239.") || dest.starts_with("255.") {
            continue;
        }
        return true;
    }
    false
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
        "meshnet" | "mesh_vpn" => {
            if let Some(iface) = detect_vpn_ip() {
                log::info!("VPNトンネル検出: {} ({})", iface.name, iface.ip);
                Ok(IpAddr::V4(iface.ip))
            } else {
                log::warn!("メッシュVPNが見つかりません。手動でIPアドレスを指定してください。");
                Err("メッシュVPNが見つかりません".to_string())
            }
        }
        "any" => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        addr => addr
            .parse::<IpAddr>()
            .map_err(|e| format!("無効なIPアドレス: {e}")),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedInterface {
    pub name: String,
    pub ip: String,
    pub kind: String, // "vpn" | "lan"
}

pub fn detect_all_interfaces() -> Vec<DetectedInterface> {
    let interfaces = match list_network_interfaces() {
        Ok(ifaces) => ifaces,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();

    for iface in &interfaces {
        if iface.ip.is_loopback() {
            continue;
        }
        if is_vpn_interface(&iface.name) {
            if has_active_routes(&iface.name, &iface.ip.to_string()) {
                result.push(DetectedInterface {
                    name: iface.name.clone(),
                    ip: iface.ip.to_string(),
                    kind: "vpn".to_string(),
                });
            }
        } else if is_private_ip(iface.ip) {
            result.push(DetectedInterface {
                name: iface.name.clone(),
                ip: iface.ip.to_string(),
                kind: "lan".to_string(),
            });
        }
    }

    result
}

#[tauri::command]
pub fn get_network_info() -> Vec<DetectedInterface> {
    detect_all_interfaces()
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
    fn test_vpn_interface_names() {
        assert!(is_vpn_interface("nordlynx"));
        assert!(is_vpn_interface("tailscale0"));
        assert!(is_vpn_interface("utun3"));
        assert!(is_vpn_interface("wg0"));
        assert!(is_vpn_interface("tun0"));
        assert!(is_vpn_interface("zt0"));
        assert!(is_vpn_interface("nebula0"));
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
    fn test_parse_routes_active_vpn() {
        let netstat = "\
Destination        Gateway            Flags    Netif
default            100.124.65.29      UGScg    utun4
100.124.65.29      100.124.65.29      UHr      utun4
100.124.65.29/32   link#24            UCS      utun4
224.0.0/4          link#24            UmCS     utun4
255.255.255.255/32 link#24            UCS      utun4";
        assert!(parse_routes_for_interface(netstat, "utun4", "100.124.65.29"));
    }

    #[test]
    fn test_parse_routes_stale_vpn() {
        // VPNオフ: 自身のIPルートとマルチキャストのみ残存
        let netstat = "\
Destination        Gateway            Flags    Netif
100.124.65.29      100.124.65.29      UHr      utun4
100.124.65.29/32   link#24            UCS      utun4
224.0.0/4          link#24            UmCS     utun4
255.255.255.255/32 link#24            UCS      utun4";
        assert!(!parse_routes_for_interface(netstat, "utun4", "100.124.65.29"));
    }

    #[test]
    fn test_parse_routes_no_matching_interface() {
        let netstat = "\
Destination        Gateway            Flags    Netif
default            192.168.1.1        UGScg    en0";
        assert!(!parse_routes_for_interface(netstat, "utun4", "100.124.65.29"));
    }

    #[test]
    fn test_parse_routes_subnet_route() {
        // Tailscale exit nodeなし: サブネットルートのみ
        let netstat = "\
Destination        Gateway            Flags    Netif
100.64/10          100.124.65.29      UGSc     utun4
100.124.65.29/32   link#24            UCS      utun4";
        assert!(parse_routes_for_interface(netstat, "utun4", "100.124.65.29"));
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
}
