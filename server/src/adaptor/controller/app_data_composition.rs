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

use crate::adaptor::gateway::app_data_gc::{
    apply_canonical_runtime_owners, build_startup_gc_request, canonical_runtime_protection,
    StdGcFileSystem,
};
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
    ///
    /// Inventory and sweeping remain on blocking workers.  Failure to load
    /// canonical runtime owners leaves workspace-keyed protection incomplete,
    /// so those deletions fail closed while independent cache/comment/process
    /// retention can still run.
    pub async fn run_startup_gc_pass(
        &self,
        shared_repo_paths: SharedRepoPaths,
        repository: Arc<dyn LocalEventTransactionRepository>,
        execution_trees: Arc<dyn crate::usecase::app_data_gc::ExecutionTreeGc>,
    ) -> Result<GcReport, String> {
        let file_system = StdGcFileSystem::with_observer(self.observer.clone());
        let inventory_file_system = file_system.clone();
        let app_data_dir = self.app_data_dir.clone();
        let inventory = crate::common::operation_context::spawn_blocking(move || {
            build_startup_gc_request(app_data_dir, shared_repo_paths, &inventory_file_system)
        })
        .await
        .map_err(|error| format!("app data gc inventory task failed: {error}"))?;
        let mut request = inventory;
        let archive_errors = crate::usecase::app_data_gc::archive_removed_execution_trees(
            request.live_worktrees.as_ref(),
            execution_trees.as_ref(),
        )
        .await
        .map_err(|error| format!("execution tree GC failed: {error}"))?;

        match crate::usecase::app_data_gc::load_canonical_runtime_owners(repository.clone()).await {
            Ok(owners) => apply_canonical_runtime_owners(&mut request, owners),
            Err(error) => {
                log::warn!(
                    "app data gc retained workspace-keyed data because canonical protection failed: {error}"
                );
            }
        }

        let plan = crate::usecase::app_data_gc::plan_startup_gc(request);
        let revalidated_runtime_protection =
            match crate::usecase::app_data_gc::load_canonical_runtime_owners(repository).await {
                Ok(owners) => canonical_runtime_protection(owners),
                Err(error) => {
                    log::warn!(
                        "app data gc retained workspace-keyed candidates because sweep-boundary canonical revalidation failed: {error}"
                    );
                    crate::usecase::app_data_gc::RuntimeProtection::incomplete()
                }
            };

        crate::common::operation_context::spawn_blocking(move || {
            let mut report = crate::usecase::app_data_gc::sweep_startup_gc(
                plan,
                revalidated_runtime_protection,
                &file_system,
            );
            report.errors += archive_errors;
            report
        })
        .await
        .map_err(|error| format!("app data gc sweep task failed: {error}"))
    }
}
