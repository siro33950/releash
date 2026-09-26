use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
use crate::domain::app_config::repository::ConfigUpdate;
use crate::domain::app_config::value_objects::AppConfigDocument;
use crate::domain::app_config::AppConfigError;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

struct Config(AppConfigDocument);
impl ConfigRepository for Config {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        Ok(self.0.clone())
    }
    fn save(&self, _: AppConfigDocument) -> Result<(), AppConfigError> {
        unreachable!()
    }
    fn update(&self, _: ConfigUpdate) -> Result<(), AppConfigError> {
        unreachable!()
    }
}

#[tokio::test]
async fn test_managed_worktree非同期解決_期限と取消の分類をruntimeまで保持する() {
    // Given
    let (dir, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let root = dir.path().to_str().unwrap().to_string();
    let mut config = crate::adaptor::gateway::app_config::config_models::config_to_domain(
        &crate::adaptor::gateway::app_config::config_models::ReleashConfig::default(),
    );
    config.app.last_repo_paths = vec![root.clone()];
    let resolver = AppConfigManagedWorktreeResolver::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(Config(config)),
    );
    for expire in [false, true] {
        let token = tokio_util::sync::CancellationToken::new();
        if !expire {
            token.cancel();
        }
        let context = OperationContext::new(
            expire.then(|| Deadline::new(std::time::Instant::now())),
            Arc::new(token),
        );
        // When
        let error =
            crate::common::operation_context::scope(context, resolver.resolve(root.clone()))
                .await
                .unwrap_err();
        let error = WorkflowRuntimeError::from(error);
        // Then
        let stopped = if expire {
            OperationStopped::Expired
        } else {
            OperationStopped::Cancelled
        };
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
        assert!(matches!(error, WorkflowRuntimeError::Technical(value) if value == stopped.into()));
    }
    assert!(resolver.resolve(root).await.is_ok());
    assert!(matches!(
        resolver
            .resolve(dir.path().join("missing").to_string_lossy().into_owned())
            .await,
        Err(ManagedWorktreeResolverError::Validation(_))
    ));
}
