#![allow(dead_code)]

use std::io::BufReader;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

const CERT_VALIDITY_DAYS: u64 = 365;

pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<TlsAcceptor, String> {
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("証明書ファイルの読み込み失敗: {e}"))?;
    let key_file = std::fs::File::open(key_path)
        .map_err(|e| format!("秘密鍵ファイルの読み込み失敗: {e}"))?;

    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("証明書のパース失敗: {e}"))?;

    let key: PrivateKeyDer = rustls_pemfile::private_key(&mut BufReader::new(key_file))
        .map_err(|e| format!("秘密鍵のパース失敗: {e}"))?
        .ok_or("秘密鍵が見つかりません")?;

    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("TLSプロトコル設定失敗: {e}"))?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| format!("TLS設定失敗: {e}"))?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn is_cert_expired(cert_path: &Path) -> bool {
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

pub fn ensure_self_signed_cert(
    ip: IpAddr,
    data_dir: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let tls_dir = data_dir.join("tls");
    std::fs::create_dir_all(&tls_dir)
        .map_err(|e| format!("TLSディレクトリ作成失敗: {e}"))?;

    let cert_path = tls_dir.join("cert.pem");
    let key_path = tls_dir.join("key.pem");
    let ip_path = tls_dir.join("bind_ip");

    let ip_matches = ip_path
        .exists()
        .then(|| std::fs::read_to_string(&ip_path).ok())
        .flatten()
        .map(|saved| saved.trim() == ip.to_string())
        .unwrap_or(false);

    if cert_path.exists() && key_path.exists() && !is_cert_expired(&cert_path) && ip_matches {
        return Ok((cert_path, key_path));
    }

    let san = rcgen::SanType::IpAddress(ip);
    let mut params =
        rcgen::CertificateParams::new(vec!["releash-server".to_string()]).map_err(|e| e.to_string())?;
    params.subject_alt_names.push(san);

    let key_pair = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("自己署名証明書の生成失敗: {e}"))?;

    std::fs::write(&cert_path, cert.pem())
        .map_err(|e| format!("証明書の書き込み失敗: {e}"))?;
    std::fs::write(&key_path, key_pair.serialize_pem())
        .map_err(|e| format!("秘密鍵の書き込み失敗: {e}"))?;
    std::fs::write(&ip_path, ip.to_string())
        .map_err(|e| format!("IP記録の書き込み失敗: {e}"))?;

    log::info!("自己署名証明書を生成しました: {}", cert_path.display());

    Ok((cert_path, key_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_is_cert_expired_nonexistent() {
        assert!(is_cert_expired(Path::new("/nonexistent/cert.pem")));
    }

    #[test]
    fn test_ensure_self_signed_cert_creates_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(100, 100, 1, 42));

        let (cert_path, key_path) = ensure_self_signed_cert(ip, dir.path()).unwrap();

        assert!(cert_path.exists());
        assert!(key_path.exists());

        let cert_content = std::fs::read_to_string(&cert_path).unwrap();
        let key_content = std::fs::read_to_string(&key_path).unwrap();
        assert!(cert_content.contains("BEGIN CERTIFICATE"));
        assert!(key_content.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_ensure_self_signed_cert_reuses_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let ip = IpAddr::V4(Ipv4Addr::new(100, 100, 1, 42));

        let (cert1, _) = ensure_self_signed_cert(ip, dir.path()).unwrap();
        let content1 = std::fs::read_to_string(&cert1).unwrap();

        let (cert2, _) = ensure_self_signed_cert(ip, dir.path()).unwrap();
        let content2 = std::fs::read_to_string(&cert2).unwrap();

        assert_eq!(content1, content2);
    }
}
