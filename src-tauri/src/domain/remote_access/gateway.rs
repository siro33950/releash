use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::domain::remote_access::{DetectedInterface, RemoteAccessError, VpnInterface};

pub trait NetworkInterfaceGateway: Send + Sync {
    fn detect_vpn_ip(&self) -> Option<VpnInterface>;
    fn detect_all_interfaces(&self) -> Vec<DetectedInterface>;
}

pub trait CertificateGateway: Send + Sync {
    fn ensure_self_signed_cert(
        &self,
        ip: IpAddr,
        data_dir: &Path,
    ) -> Result<(PathBuf, PathBuf), RemoteAccessError>;
}

pub trait QrRenderGateway: Send + Sync {
    fn generate_qr_svg(&self, data: &str) -> Result<String, RemoteAccessError>;
}
