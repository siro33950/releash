use std::ffi::OsString;
use std::fs;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::domain::agent_session::aggregates::{ProviderExecutable, ProviderUnavailableReason};
use crate::domain::agent_session::ProviderExecutableProbeGateway;

use super::LocalProviderExecutableProbeGateway;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::infrastructure::process::search_path::{LoginShellPathError, SearchPathSource};

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct FailingSearchPathSource;

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct FixedSearchPathSource(OsString);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl SearchPathSource for FailingSearchPathSource {
    fn load(&self) -> Result<OsString, LoginShellPathError> {
        Err(LoginShellPathError::Spawn)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl SearchPathSource for FixedSearchPathSource {
    fn load(&self) -> Result<OsString, LoginShellPathError> {
        Ok(self.0.clone())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn test_provider_availability_初期login_shell_path取得失敗をpath探索不能として公開する() {
    let gateway = LocalProviderExecutableProbeGateway::with_initial_search_path(Err(
        LoginShellPathError::Timeout,
    ));

    assert_eq!(
        gateway
            .resolve(&ProviderExecutable::new("missing-cli").unwrap())
            .unavailable_reason(),
        Some(ProviderUnavailableReason::SearchPathUnavailable)
    );
    assert_eq!(
        gateway
            .resolve(&ProviderExecutable::new("/missing/absolute-cli").unwrap())
            .unavailable_reason(),
        Some(ProviderUnavailableReason::NotFound)
    );
}

#[test]
fn test_provider_availability_probe_設定された実行可能fileをresolvedに変換する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = LocalProviderExecutableProbeGateway::with_search_path(None);

    let availability = gateway
        .resolve(&ProviderExecutable::new(executable.to_string_lossy().into_owned()).unwrap());

    assert_eq!(
        availability.resolved_executable().unwrap().as_path(),
        executable.as_path()
    );
}

#[test]
fn test_provider_availability_probe_path上のcommandと利用不可理由を変換する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = LocalProviderExecutableProbeGateway::with_search_path(Some(
        temporary.path().as_os_str().to_os_string(),
    ));

    assert!(gateway
        .resolve(&ProviderExecutable::new("agent-cli").unwrap())
        .resolved_executable()
        .is_some());
    assert_eq!(
        gateway
            .resolve(&ProviderExecutable::new("missing-cli").unwrap())
            .unavailable_reason(),
        Some(ProviderUnavailableReason::NotFound)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_provider_availability_probe_non_utf8_pathを同一os_pathのまま解決する() {
    use std::os::unix::ffi::OsStringExt;

    let temporary = tempfile::tempdir().unwrap();
    let bin = temporary
        .path()
        .join(std::ffi::OsString::from_vec(b"bin-\xff".to_vec()));
    fs::create_dir(&bin).unwrap();
    let executable = bin.join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway =
        LocalProviderExecutableProbeGateway::with_search_path(Some(bin.as_os_str().to_os_string()));

    let availability = gateway.resolve(&ProviderExecutable::new("agent-cli").unwrap());

    assert_eq!(
        availability.resolved_executable().unwrap().as_path(),
        executable
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn test_provider_availability_refresh_shell取得失敗を返し既存pathを維持する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = LocalProviderExecutableProbeGateway::with_search_path_source(
        Some(temporary.path().as_os_str().to_os_string()),
        Arc::new(FailingSearchPathSource),
    );
    assert_eq!(
        gateway.refresh_search_path(),
        Err(crate::domain::agent_session::ProviderExecutableProbeGatewayError::RefreshFailed)
    );
    assert!(gateway
        .resolve(&ProviderExecutable::new("agent-cli").unwrap())
        .resolved_executable()
        .is_some());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn test_provider_availability_refresh_取得pathをlocal_stateへ反映する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = LocalProviderExecutableProbeGateway::with_search_path_source(
        None,
        Arc::new(FixedSearchPathSource(
            temporary.path().as_os_str().to_os_string(),
        )),
    );

    gateway.refresh_search_path().unwrap();

    assert!(gateway
        .resolve(&ProviderExecutable::new("agent-cli").unwrap())
        .resolved_executable()
        .is_some());
}
