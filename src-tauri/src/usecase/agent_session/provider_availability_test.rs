use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ProviderUnavailableReason, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository, ProviderExecutableProbeGateway,
    ProviderExecutableProbeGatewayError,
};
use crate::domain::provider_lifecycle::ProviderKind;

use super::{ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError};

use super::super::test_helpers::{
    FakeProviderExecutableConfigRepository, FakeProviderExecutableProbeGateway,
};

#[test]
fn test_provider利用可否_初期化時にconfigとprobeから全providerのsnapshotを構築する() {
    // Given
    let config = Arc::new(FakeProviderExecutableConfigRepository::with_override(
        ProviderKind::Claude,
        "/custom/claude",
    ));
    // When
    let availability = ProviderAvailabilityUsecase::initialize(
        config,
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();
    let snapshot = availability.snapshot().unwrap();

    // Then
    assert_eq!(snapshot.entries().len(), ProviderKind::supported().len());
    let claude = snapshot.entry(ProviderKind::Claude);
    assert_eq!(
        claude
            .configured_executable()
            .map(ProviderExecutable::as_str),
        Some("/custom/claude")
    );
    assert_eq!(
        claude
            .resolved_executable()
            .map(ResolvedProviderExecutable::as_path),
        Some(std::path::Path::new("/custom/claude"))
    );
    let codex = snapshot.entry(ProviderKind::Codex);
    assert_eq!(codex.effective_executable().as_str(), "codex");
    assert_eq!(
        codex
            .resolved_executable()
            .map(ResolvedProviderExecutable::as_path),
        Some(std::path::Path::new("/resolved/codex"))
    );
}

#[test]
fn test_provider利用可否_購読用出力にproviderと利用不可理由を写す() {
    // Given
    let availability = ProviderAvailabilityUsecase::initialize(
        Arc::new(FakeProviderExecutableConfigRepository::with_override(
            ProviderKind::Codex,
            "missing-codex",
        )),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();

    // When
    let snapshot = availability.snapshot_dto().unwrap();

    // Then
    assert_eq!(
        snapshot.providers[0].provider,
        crate::usecase::provider_dto::AgentSessionProviderDto::Claude
    );
    assert_eq!(
        snapshot.providers[1].provider,
        crate::usecase::provider_dto::AgentSessionProviderDto::Codex
    );
    assert_eq!(
        snapshot.providers[1].configured_executable.as_deref(),
        Some("missing-codex")
    );
    assert_eq!(
        snapshot.providers[1].unavailable_reason,
        Some(super::ProviderUnavailableReasonDto::NotFound)
    );
    assert!(!snapshot.providers[1].available);
}

#[test]
fn test_provider_availability_利用可能候補とlaunch実行fileを同じsnapshotから返す() {
    let availability = ProviderAvailabilityUsecase::initialize(
        Arc::new(FakeProviderExecutableConfigRepository::with_override(
            ProviderKind::Codex,
            "missing-codex",
        )),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();

    assert_eq!(
        availability.available_providers().unwrap(),
        vec![ProviderKind::Claude]
    );
    assert_eq!(
        ProviderAvailabilityReader::resolved_executable(&availability, ProviderKind::Claude)
            .unwrap()
            .as_path(),
        std::path::Path::new("/resolved/claude")
    );
    assert!(
        ProviderAvailabilityReader::resolved_executable(&availability, ProviderKind::Codex)
            .is_none()
    );
}

#[test]
fn test_provider利用可否_updateは正規化した設定と保存revisionを購読へ渡す() {
    // Given
    let config = Arc::new(FakeProviderExecutableConfigRepository::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        config.clone(),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();

    // When
    availability
        .update_configured_executable(ProviderKind::Claude, " /custom/claude ")
        .unwrap();

    // Then
    let before = availability.snapshot_dto().unwrap();
    availability
        .update_configured_executable(ProviderKind::Claude, " /custom/claude ")
        .unwrap();
    let after = availability.snapshot_dto().unwrap();
    assert_eq!(
        before.providers[0].configured_executable,
        after.providers[0].configured_executable
    );
    assert_eq!(
        after.providers[0].configuration_revision,
        before.providers[0].configuration_revision + 1
    );
    assert_eq!(after.providers[1], before.providers[1]);
    assert_eq!(
        config
            .configured_executable(ProviderKind::Claude)
            .unwrap()
            .unwrap()
            .as_str(),
        "/custom/claude"
    );
}

#[test]
fn test_provider利用可否_updateは保存後に対象を再判定しresetでdefaultへ戻す() {
    // Given
    let config = Arc::new(FakeProviderExecutableConfigRepository::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        config.clone(),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();

    // When
    availability
        .update_configured_executable(ProviderKind::Claude, "/custom/claude")
        .unwrap();
    let updated = availability.snapshot().unwrap();
    // Then
    assert_eq!(
        updated
            .entry(ProviderKind::Claude)
            .configured_executable()
            .map(ProviderExecutable::as_str),
        Some("/custom/claude")
    );
    assert_eq!(
        config
            .configured_executable(ProviderKind::Claude)
            .unwrap()
            .unwrap()
            .as_str(),
        "/custom/claude"
    );

    // When
    availability
        .reset_configured_executable(ProviderKind::Claude)
        .unwrap();
    let reset = availability.snapshot().unwrap();
    // Then
    assert_eq!(
        reset.entry(ProviderKind::Claude).configured_executable(),
        None
    );
    assert_eq!(
        reset
            .entry(ProviderKind::Claude)
            .effective_executable()
            .as_str(),
        "claude"
    );
}

#[test]
fn test_provider_availability_保存失敗時はregistryを変更しない() {
    let config = Arc::new(FakeProviderExecutableConfigRepository::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        config.clone(),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();
    let before = availability.snapshot().unwrap();
    config.fail_save();

    assert_eq!(
        availability
            .update_configured_executable(ProviderKind::Codex, "/custom/codex")
            .unwrap_err(),
        ProviderAvailabilityUsecaseError::Config(
            crate::domain::agent_session::ProviderExecutableConfigRepositoryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into()
                }
            )
        )
    );
    assert_eq!(availability.snapshot().unwrap(), before);
}

#[test]
fn test_provider利用可否_refreshは探索環境更新後に全providerを一括再判定する() {
    // Given
    let probe = Arc::new(FakeProviderExecutableProbeGateway::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        Arc::new(FakeProviderExecutableConfigRepository::default()),
        probe.clone(),
    )
    .unwrap();
    assert_eq!(availability.available_providers().unwrap().len(), 2);
    probe.set_force_missing(true);

    // When
    availability.refresh().unwrap();
    let refreshed = availability.snapshot().unwrap();
    let current = availability.snapshot().unwrap();

    // Then
    assert_eq!(*probe.refreshes.lock().unwrap(), 1);
    assert!(refreshed
        .entries()
        .iter()
        .all(|entry| !entry.is_available()));
    assert!(current.entries().iter().all(|entry| !entry.is_available()));
}

struct BlockingProviderExecutableProbeGateway {
    block_next: AtomicBool,
    entered: Barrier,
    release: Barrier,
}

impl BlockingProviderExecutableProbeGateway {
    fn new() -> Self {
        Self {
            block_next: AtomicBool::new(false),
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

impl ProviderExecutableProbeGateway for BlockingProviderExecutableProbeGateway {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability {
        if self.block_next.swap(false, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
            ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
        } else if executable.as_str() == "codex" {
            ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
        } else {
            ProviderAvailability::available(
                ResolvedProviderExecutable::new(
                    format!("/resolved/{}", executable.as_str()).into(),
                )
                .unwrap(),
            )
        }
    }

    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError> {
        Ok(())
    }
}

#[test]
fn test_provider利用可否_refresh中のreadへ部分更新snapshotを公開しない() {
    // Given
    let probe = Arc::new(BlockingProviderExecutableProbeGateway::new());
    let availability = Arc::new(
        ProviderAvailabilityUsecase::initialize(
            Arc::new(FakeProviderExecutableConfigRepository::default()),
            probe.clone(),
        )
        .unwrap(),
    );
    let before = availability.snapshot().unwrap();
    probe.block_next.store(true, Ordering::SeqCst);
    // When
    let refreshing = {
        let availability = availability.clone();
        std::thread::spawn(move || availability.refresh().unwrap())
    };

    probe.entered.wait();
    let during = availability.snapshot().unwrap();
    probe.release.wait();
    refreshing.join().unwrap();
    let after = availability.snapshot().unwrap();
    let current = availability.snapshot().unwrap();

    // Then
    assert_eq!(during, before);
    assert_eq!(current, after);
    assert!(after.entries().iter().all(|entry| !entry.is_available()));
}

#[test]
fn test_provider設定_更新とresetとrefresh成功時だけ購読へ通知する() {
    // Given
    let config = Arc::new(FakeProviderExecutableConfigRepository::default());
    let publisher = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let usecase = ProviderAvailabilityUsecase::initialize(
        config.clone(),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap()
    .with_state_publisher(publisher);
    // When
    usecase
        .update_configured_executable(ProviderKind::Codex, "/custom/codex")
        .unwrap();
    let updated = changes.try_recv().unwrap();
    // Then
    assert_eq!(
        updated,
        crate::usecase::state_subscription::StateChangeSource::Providers
    );
    // When
    usecase
        .reset_configured_executable(ProviderKind::Codex)
        .unwrap();
    let reset = changes.try_recv().unwrap();
    // Then
    assert_eq!(
        reset,
        crate::usecase::state_subscription::StateChangeSource::Providers
    );
    // When
    usecase.refresh().unwrap();
    let refreshed = changes.try_recv().unwrap();
    // Then
    assert_eq!(
        refreshed,
        crate::usecase::state_subscription::StateChangeSource::Providers
    );
    // Given
    config.fail_save();
    // When
    let failed = usecase
        .update_configured_executable(ProviderKind::Codex, "/custom/codex")
        .is_err();
    let no_change = changes.try_recv().is_err();
    // Then
    assert!(failed);
    assert!(no_change);
}

#[test]
fn test_provider利用可否_四種類の利用不可理由を出力へ写す() {
    // Given
    let cases = [
        (
            ProviderUnavailableReason::NotFound,
            super::ProviderUnavailableReasonDto::NotFound,
        ),
        (
            ProviderUnavailableReason::NotExecutable,
            super::ProviderUnavailableReasonDto::NotExecutable,
        ),
        (
            ProviderUnavailableReason::SearchPathUnavailable,
            super::ProviderUnavailableReasonDto::SearchPathUnavailable,
        ),
        (
            ProviderUnavailableReason::ProbeFailed,
            super::ProviderUnavailableReasonDto::ProbeFailed,
        ),
    ];

    // When
    for (domain, expected) in cases {
        let actual = super::ProviderUnavailableReasonDto::from(domain);
        // Then
        assert_eq!(actual, expected);
    }
}

#[test]
fn test_provider設定失敗_全変種から技術的な失敗だけを参照する() {
    use super::ProviderAvailabilityUsecaseError as E;
    use crate::domain::agent_session::{
        ProviderExecutableConfigRepositoryError as C, ProviderExecutableProbeGatewayError as P,
    };
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given / When / Then
    for error in [E::InvalidInput, E::Corrupt, E::Config(C::InvalidInput)] {
        assert_eq!(error.technical_failure(), None);
    }
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "source failure".into(),
        };
        for error in [
            E::Config(C::Technical(failure.clone())),
            E::Refresh(P::Technical(failure.clone())),
        ] {
            assert_eq!(error.technical_failure(), Some(&failure));
        }
    }
}
