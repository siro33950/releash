use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::domain::remote_access::{CertificateGateway, RemoteAccessError};

pub fn ensure_self_signed_cert(
    gateway: &dyn CertificateGateway,
    ip: IpAddr,
    data_dir: &Path,
) -> Result<(PathBuf, PathBuf), RemoteAccessError> {
    gateway.ensure_self_signed_cert(ip, data_dir)
}
