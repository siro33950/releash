use crate::domain::daemon::{
    Daemon, DaemonIdentity, DaemonInfo, DaemonRepository, DaemonRequest, StartupFailure,
    StopAcceptance, StopRequest,
};
pub struct InMemoryDaemonRepository {
    daemon: parking_lot::Mutex<Daemon>,
    commands: tokio::sync::RwLock<()>,
}
impl InMemoryDaemonRepository {
    pub(crate) fn new(
        identity: DaemonIdentity,
        release: String,
        protocol: u32,
        cli_installation: crate::domain::installation::CliInstallation,
    ) -> Self {
        Self {
            daemon: parking_lot::Mutex::new(Daemon::new(
                identity,
                release,
                protocol,
                cli_installation,
            )),
            commands: tokio::sync::RwLock::new(()),
        }
    }
    pub(crate) async fn drain_commands(&self) {
        let _commands = self.commands.write().await;
    }
    pub async fn admission(&self) -> DaemonAdmissionGuard<'_> {
        DaemonAdmissionGuard {
            _commands: self.commands.read().await,
            repository: self,
        }
    }
}
pub struct DaemonAdmissionGuard<'a> {
    _commands: tokio::sync::RwLockReadGuard<'a, ()>,
    repository: &'a InMemoryDaemonRepository,
}
impl DaemonAdmissionGuard<'_> {
    pub(crate) fn if_admitted<T>(
        &self,
        request: DaemonRequest,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let daemon = self.repository.daemon.lock();
        daemon.admits(request).then(operation)
    }
    pub fn admits(&self, request: DaemonRequest) -> bool {
        self.repository.daemon.lock().admits(request)
    }
}
#[async_trait::async_trait]
impl DaemonRepository for InMemoryDaemonRepository {
    async fn info(&self) -> DaemonInfo {
        self.daemon.lock().info()
    }
    async fn admits(&self, request: DaemonRequest) -> bool {
        self.daemon.lock().admits(request)
    }
    async fn serve(&self) {
        self.daemon.lock().serve();
    }
    async fn fail(&self, failure: StartupFailure) {
        self.daemon.lock().fail(failure);
    }
    async fn stop(&self, request: StopRequest) -> StopAcceptance {
        self.daemon.lock().stop(request)
    }
    async fn stopped(&self) {
        self.daemon.lock().stopped();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn serving() -> std::sync::Arc<InMemoryDaemonRepository> {
    serving_with_identity(DaemonIdentity {
        daemon_id: "test-daemon".into(),
        pid: 1,
        process_started_at: 1,
    })
}
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn serving_with_identity(
    identity: DaemonIdentity,
) -> std::sync::Arc<InMemoryDaemonRepository> {
    let repository = std::sync::Arc::new(InMemoryDaemonRepository::new(
        identity,
        env!("CARGO_PKG_VERSION").into(),
        1,
        crate::domain::installation::CliInstallation::Allowed,
    ));
    repository.daemon.lock().serve();
    repository
}
#[cfg(test)]
#[path = "daemon_test.rs"]
pub(crate) mod daemon_tests;

impl From<crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError>
    for crate::domain::daemon::StartupFailureKind
{
    fn from(
        error: crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
    ) -> Self {
        use crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
        use crate::domain::daemon::StartupFailureKind as K;

        match error {
            E::WriterLockHeld => K::StoreInUse,
            E::StorageUnavailable(failure) => K::StorageUnavailable(failure.nature),
            E::UnsupportedRuntime => K::UnsupportedRuntime,
            E::UnsupportedStoreVersion => K::UnsupportedStoreVersion,
            E::InitializationStateInvalid => K::InitializationStateInvalid,
            E::StoreValidationFailed => K::StoreValidationFailed,
            E::SchemaEvolutionFailed => K::SchemaEvolutionFailed,
        }
    }
}
