use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::domain::remote_access::value_objects::RawInterface;
use crate::domain::remote_access::{DetectedInterface, VpnInterface};

const VPN_INTERFACE_PREFIXES: &[&str] =
    &["nordlynx", "tailscale", "utun", "wg", "tun", "zt", "nebula"];
const CERT_VALIDITY_DAYS: u64 = 365;

pub fn is_vpn_interface(name: &str) -> bool {
    let lower = name.to_lowercase();
    VPN_INTERFACE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

pub fn is_private_ip(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    match octets[0] {
        10 => true,
        172 => (16..=31).contains(&octets[1]),
        192 => octets[1] == 168,
        _ => false,
    }
}

pub fn parse_routes_for_interface(netstat_output: &str, iface_name: &str, own_ip: &str) -> bool {
    for line in netstat_output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[3] != iface_name {
            continue;
        }
        let dest = fields[0];
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

pub fn parse_ifconfig_output(output: &str) -> Result<Vec<RawInterface>, String> {
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

pub fn select_vpn_interface(
    interfaces: Vec<RawInterface>,
    has_active_routes: impl Fn(&str, &str) -> bool,
) -> Option<VpnInterface> {
    let candidate = interfaces
        .into_iter()
        .find(|iface| is_vpn_interface(&iface.name))?;

    if !has_active_routes(&candidate.name, &candidate.ip.to_string()) {
        return None;
    }

    Some(VpnInterface {
        name: candidate.name,
        ip: candidate.ip,
    })
}

pub fn classify_interfaces(
    interfaces: &[RawInterface],
    has_active_routes: impl Fn(&str, &str) -> bool,
) -> Vec<DetectedInterface> {
    let mut result = Vec::new();

    for iface in interfaces {
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

pub fn build_connection_url(bind: &str, port: u16, tls_enabled: bool) -> String {
    let scheme = if tls_enabled { "https" } else { "http" };
    format!("{scheme}://{bind}:{port}")
}

pub fn is_cert_expired(cert_path: &Path) -> bool {
    let metadata = match std::fs::metadata(cert_path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return true,
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    age > Duration::from_secs(CERT_VALIDITY_DAYS * 24 * 60 * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpn_interface_names() {
        assert!(is_vpn_interface("nordlynx"));
        assert!(is_vpn_interface("tailscale0"));
        assert!(is_vpn_interface("utun3"));
        assert!(!is_vpn_interface("en0"));
    }

    #[test]
    fn test_parse_ifconfig_with_vpn() {
        let output = r#"en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	inet 192.168.1.100 netmask 0xffffff00 broadcast 192.168.1.255
utun3: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1280
	inet 100.100.1.42 --> 100.100.1.42 netmask 0xffffffff
"#;
        let interfaces = parse_ifconfig_output(output).unwrap();
        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces[1].ip, Ipv4Addr::new(100, 100, 1, 42));
    }

    #[test]
    fn test_parse_routes_active_vpn() {
        let netstat = "\
Destination        Gateway            Flags    Netif
default            100.124.65.29      UGScg    utun4
100.124.65.29/32   link#24            UCS      utun4";
        assert!(parse_routes_for_interface(
            netstat,
            "utun4",
            "100.124.65.29"
        ));
    }

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip(Ipv4Addr::new(192, 168, 1, 100)));
        assert!(is_private_ip(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ip(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(!is_private_ip(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn test_build_url_with_tls() {
        assert_eq!(
            build_connection_url("127.0.0.1", 9700, true),
            "https://127.0.0.1:9700"
        );
    }
}
