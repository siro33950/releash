use super::*;
use crate::domain::installation::{BuildKind, CliInstallation};
use crate::usecase::installation::{installation_tests::FakeInstallation, InstallationUsecase};
use std::sync::Arc;

#[tokio::test]
async fn test_設置rpc_成功と設置済みを返し配置拒否はfailed_preconditionにする() {
    // Given
    let port = Arc::new(FakeInstallation::default());
    let mut deps = crate::acceptance_test_support::build_client_dependencies("/unused".into());
    deps.installation_usecase = Some(Arc::new(InstallationUsecase(port)));
    let mut router = ClientCommandDispatch::new(deps.daemon.clone());
    register_shared(&mut router, &deps);
    // When / Then
    for status in [
        wire::CliInstallStatus::Installed,
        wire::CliInstallStatus::AlreadyInstalled,
    ] {
        let wire::command_result::Command::InstallCli(result) = router
            .dispatch(wire::command_request::Command::InstallCli(
                wire::InstallCliRequest {},
            ))
            .await
            .unwrap()
        else {
            panic!("installation result");
        };
        assert_eq!(result.status, status as i32);
        assert_eq!(result.path, "/usr/local/bin/releash");
    }
    for (fake, expected) in [
        (
            FakeInstallation {
                build: BuildKind::Development,
                ..Default::default()
            },
            CliInstallation::Development,
        ),
        (
            FakeInstallation {
                read_only: true,
                ..Default::default()
            },
            CliInstallation::ReadOnly,
        ),
        (
            FakeInstallation {
                path: "/AppTranslocation/id/releashd".into(),
                ..Default::default()
            },
            CliInstallation::Translocated,
        ),
    ] {
        let port = Arc::new(fake);
        deps.installation_usecase = Some(Arc::new(InstallationUsecase(port.clone())));
        let mut router = ClientCommandDispatch::new(deps.daemon.clone());
        register_shared(&mut router, &deps);
        let error = router
            .dispatch(wire::command_request::Command::InstallCli(
                wire::InstallCliRequest {},
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind, connectrpc::ErrorCode::FailedPrecondition);
        assert_eq!(error.message.as_deref(), Some(expected.reason()));
        assert_eq!(*port.calls.lock(), ["location"]);
    }
}
#[tokio::test]
async fn test_ログイン判定rpc_画面のパスを使い相対パスを拒否する() {
    // Given
    let mut deps = crate::acceptance_test_support::build_client_dependencies("/unused".into());
    deps.installation_usecase = Some(Arc::new(InstallationUsecase(Arc::new(
        FakeInstallation::default(),
    ))));
    let mut router = ClientCommandDispatch::new(deps.daemon.clone());
    register_shared(&mut router, &deps);
    // When / Then
    for (path, status) in [
        (
            "/Applications/Releash.app/releash-desktop",
            wire::LoginRegistrationStatus::Allowed,
        ),
        (
            "/AppTranslocation/gui/releash-desktop",
            wire::LoginRegistrationStatus::Translocated,
        ),
    ] {
        let wire::command_result::Command::CheckLoginRegistration(result) = router
            .dispatch(wire::command_request::Command::CheckLoginRegistration(
                wire::CheckLoginRegistrationRequest {
                    executable_path: path.into(),
                },
            ))
            .await
            .unwrap()
        else {
            panic!("registration result");
        };
        assert_eq!(result.status, status as i32);
        assert_eq!(
            result.reason.is_empty(),
            status == wire::LoginRegistrationStatus::Allowed
        );
    }
    let error = router
        .dispatch(wire::command_request::Command::CheckLoginRegistration(
            wire::CheckLoginRegistrationRequest {
                executable_path: "relative".into(),
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, connectrpc::ErrorCode::InvalidArgument);
}
