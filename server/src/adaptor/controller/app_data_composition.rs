//! Production app-data composition boundary.
//!
//! Issue #1499 B-070 is a lifecycle-wide constraint, not a property of the
//! SQLite adapter in isolation.  This composition owns the one app-data root
//! and the one path observer supplied to every app-data collaborator used by
//! startup maintenance: the fixed SQLite store and
//! issue #1372 GC/retention.  Production installs the no-op observer; the
//! acceptance composition replaces it once here and therefore cannot
//! accidentally test an independently hand-wired set of adapters.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::app_data_gc::{log_gc_report, StartupInventory, StdGcFileSystem};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
use crate::domain::app_data_gc::GcReport;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::infrastructure::app_data_path::{AppDataPathObserver, NoopAppDataPathObserver};

#[derive(Clone)]
pub struct ProductionAppDataComposition {
    app_data_dir: PathBuf,
    observer: Arc<dyn AppDataPathObserver>,
    retry_limiter: Arc<crate::common::retry::RetryLimiter>,
}

impl ProductionAppDataComposition {
    pub fn new(
        app_data_dir: PathBuf,
        retry_limiter: Arc<crate::common::retry::RetryLimiter>,
    ) -> Self {
        Self {
            app_data_dir,
            retry_limiter,
            observer: Arc::new(NoopAppDataPathObserver),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_observer(app_data_dir: PathBuf, observer: Arc<dyn AppDataPathObserver>) -> Self {
        Self {
            app_data_dir,
            observer,
            retry_limiter: std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        }
    }

    pub fn open_local_event_store(
        &self,
    ) -> Result<
        Arc<LocalEventStore>,
        crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
    > {
        let mut config = LocalEventStoreConfig::production(
            self.app_data_dir.clone(),
            self.retry_limiter.clone(),
        );
        config.path_observer = self.observer.clone();
        LocalEventStore::open(config)
    }

    /// Execute the exact production GC/retention pass.
    pub async fn run_startup_gc_pass(
        &self,
        shared_repo_paths: SharedRepoPaths,
        repository: Arc<dyn LocalEventTransactionRepository>,
        execution_trees: Arc<dyn crate::usecase::app_data_gc::ExecutionTreeGc>,
        workspace_states: Arc<dyn crate::domain::workspace_state::WorkspaceStateRepository>,
    ) -> Result<GcReport, crate::usecase::app_data_gc::StartupGcFailure> {
        let file_system = StdGcFileSystem::with_observer(self.observer.clone());
        let inventory = StartupInventory {
            app_data_dir: self.app_data_dir.clone(),
            shared_repo_paths,
            file_system: file_system.clone(),
        };
        crate::common::telemetry::observe_result_async(
            crate::usecase::app_data_gc::execute_startup_gc(
                &inventory,
                repository,
                execution_trees.as_ref(),
                workspace_states.as_ref(),
                Arc::new(file_system),
            ),
            |result, _| match result {
                Ok(report) => {
                    log_gc_report(report);
                    log::info!("{}", report.log_summary());
                }
                Err(failure) => log_gc_report(&failure.report),
            },
        )
        .await
    }
}
