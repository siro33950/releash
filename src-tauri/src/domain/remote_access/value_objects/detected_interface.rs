use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct VpnInterface {
    pub name: String,
    pub ip: Ipv4Addr,
}

#[derive(Debug, Clone)]
pub struct RawInterface {
    pub name: String,
    pub ip: Ipv4Addr,
}

#[derive(Debug, Clone)]
pub struct DetectedInterface {
    pub name: String,
    pub ip: String,
    pub kind: String,
}
