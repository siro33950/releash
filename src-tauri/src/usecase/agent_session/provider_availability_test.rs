use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ProviderUnavailableReason, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository,
    ProviderExecutableConfigRepositoryError, ProviderExecutableProbeGateway,
    ProviderExecutableProbeGatewayError,
};
use crate::domain::provider_lifecycle::ProviderKind;

use super::{ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError};

#[derive(Default)]
struct FakeProviderExecutableConfigRepository {
    overrides: Mutex<HashMap<ProviderKind, ProviderExecutable>>,
    fail_save: AtomicBool,
}

impl FakeProviderExecutableConfigRepository {
    fn with_override(provider: ProviderKind, executable: &str) -> Self {
        Self {
            overrides: Mutex::new(HashMap::from([(
                provider,
                ProviderExecutable::new(executable).unwrap(),
            )])),
            fail_save: AtomicBool::new(false),
        }
    }

    fn fail_save(&self) {
        self.fail_save.store(true, Ordering::SeqCst);
    }
}

impl ProviderExecutableConfigRepository for FakeProviderExecutableConfigRepository {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError> {
        Ok(self.overrides.lock().unwrap().get(&provider).cloned())
    }

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError> {
        if self.fail_save.load(Ordering::SeqCst) {
            return Err(ProviderExecutableConfigRepositoryError::Unavailable);
        }
        let mut overrides = self.overrides.lock().unwrap();
        match executable {
            Some(executable) => {
                overrides.insert(provider, executable.clone());
            }
            None => {
                overrides.remove(&provider);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeProviderExecutableProbeGateway {
    force_missing: AtomicBool,
    refreshes: Mutex<usize>,
}

impl FakeProviderExecutableProbeGateway {
    fn set_force_missing(&self, force_missing: bool) {
        self.force_missing.store(force_missing, Ordering::SeqCst);
    }
}

impl ProviderExecutableProbeGateway for FakeProviderExecutableProbeGateway {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability {
        if self.force_missing.load(Ordering::SeqCst) || executable.as_str().contains("missing") {
            ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
        } else {
            let resolved = if executable.as_str().starts_with('/') {
                executable.as_str().into()
            } else {
                format!("/resolved/{}", executable.as_str()).into()
            };
            ProviderAvailability::available(ResolvedProviderExecutable::new(resolved).unwrap())
        }
    }

    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError> {
        *self.refreshes.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn test_provider_availability_初期化時にconfigとprobeから全providerのsnapshotを構築する() {
    let config = Arc::new(FakeProviderExecutableConfigRepository::with_override(
        ProviderKind::Claude,
        "/custom/claude",
    ));
    let availability = ProviderAvailabilityUsecase::initialize(
        config,
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();
    let snapshot = availability.snapshot().unwrap();

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
fn test_provider_availability_updateは保存後に対象を再判定しresetでdefaultへ戻す() {
    let config = Arc::new(FakeProviderExecutableConfigRepository::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        config.clone(),
        Arc::new(FakeProviderExecutableProbeGateway::default()),
    )
    .unwrap();

    let updated = availability
        .update_configured_executable(ProviderKind::Claude, "/custom/claude")
        .unwrap();
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

    let reset = availability
        .reset_configured_executable(ProviderKind::Claude)
        .unwrap();
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
        ProviderAvailabilityUsecaseError::ConfigUnavailable
    );
    assert_eq!(availability.snapshot().unwrap(), before);
}

#[test]
fn test_provider_availability_refreshは探索環境更新後に全providerを一括再判定する() {
    let probe = Arc::new(FakeProviderExecutableProbeGateway::default());
    let availability = ProviderAvailabilityUsecase::initialize(
        Arc::new(FakeProviderExecutableConfigRepository::default()),
        probe.clone(),
    )
    .unwrap();
    assert_eq!(availability.available_providers().unwrap().len(), 2);
    probe.set_force_missing(true);

    let refreshed = availability.refresh().unwrap();

    assert_eq!(*probe.refreshes.lock().unwrap(), 1);
    assert!(refreshed
        .entries()
        .iter()
        .all(|entry| !entry.is_available()));
    assert!(availability
        .snapshot()
        .unwrap()
        .entries()
        .iter()
        .all(|entry| !entry.is_available()));
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
fn test_provider_availability_refresh中のreadへ部分更新snapshotを公開しない() {
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
    let refreshing = {
        let availability = availability.clone();
        std::thread::spawn(move || availability.refresh().unwrap())
    };

    probe.entered.wait();
    assert_eq!(availability.snapshot().unwrap(), before);
    probe.release.wait();
    let after = refreshing.join().unwrap();

    assert_eq!(availability.snapshot().unwrap(), after);
    assert!(after.entries().iter().all(|entry| !entry.is_available()));
}
