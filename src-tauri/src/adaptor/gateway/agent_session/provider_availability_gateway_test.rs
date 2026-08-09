use std::fs;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::domain::agent_session::ProviderAvailabilityGateway;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{
    ProviderAgentSessionProviderDto, ProviderAvailabilityQueryService,
};

use super::{LocalProviderAvailabilityGateway, LocalProviderAvailabilityQueryService};

#[test]
fn test_provider_availability_設定された実行可能fileだけを利用可能と判定する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let unavailable = temporary.path().join("missing-cli");
    let gateway = LocalProviderAvailabilityGateway::new(
        executable.to_string_lossy().into_owned(),
        unavailable.to_string_lossy().into_owned(),
    );

    assert!(gateway.is_available(ProviderKind::Claude));
    assert!(!gateway.is_available(ProviderKind::Codex));
}

#[test]
fn test_provider_availability_path上の実行可能commandを利用可能と判定する() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let gateway = LocalProviderAvailabilityGateway::with_search_path(
        "agent-cli".to_string(),
        "missing-cli".to_string(),
        Some(temporary.path().as_os_str().to_os_string()),
    );

    assert!(gateway.is_available(ProviderKind::Claude));
    assert!(!gateway.is_available(ProviderKind::Codex));
}

#[test]
fn test_provider_availability_query_利用可能なproviderだけをdtoで返す() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("agent-cli");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let availability = Arc::new(LocalProviderAvailabilityGateway::new(
        temporary
            .path()
            .join("missing-cli")
            .to_string_lossy()
            .into_owned(),
        executable.to_string_lossy().into_owned(),
    ));
    let query = LocalProviderAvailabilityQueryService::new(availability);

    assert_eq!(
        query.available_providers(),
        vec![ProviderAgentSessionProviderDto::Codex]
    );
}
