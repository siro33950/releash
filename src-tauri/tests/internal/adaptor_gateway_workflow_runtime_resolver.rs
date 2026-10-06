use releash_lib::test_support::integration::platform::Deadline;
use releash_lib::test_support::integration::platform::OperationContext;
use releash_lib::test_support::integration::platform::OperationStopped;
use releash_lib::test_support::integration::repository::ConfigRepository;
use releash_lib::test_support::integration::repository::ConfigUpdate;
use releash_lib::test_support::integration::settings::AppConfigDocument;
use releash_lib::test_support::integration::settings::AppConfigError;
use releash_lib::test_support::integration::transport::ConnectFailure;
use releash_lib::test_support::integration::workflow::AppConfigManagedWorktreeResolver;
use releash_lib::test_support::integration::workflow::ManagedWorktreeResolver;
use releash_lib::test_support::integration::workflow::ManagedWorktreeResolverError;
use releash_lib::test_support::integration::workflow::WorkflowRuntimeError;
use std::sync::Arc;

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
pub async fn test_managed_worktree非同期解決_期限と取消の分類をruntimeまで保持する() {
    // Given
    let (dir, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let root = dir.path().to_str().unwrap().to_string();
    let mut config = releash_lib::test_support::integration::settings::config_to_domain(
        &releash_lib::test_support::integration::settings::ReleashConfig::default(),
    );
    config.app.last_repo_paths = vec![root.clone()];
    let resolver = AppConfigManagedWorktreeResolver::new(
        Arc::new(releash_lib::test_support::integration::platform::build_repository_usecase()),
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
        let error = releash_lib::test_support::integration::platform::scope(
            context,
            resolver.resolve(root.clone()),
        )
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
            releash_lib::test_support::integration::platform::TechnicalFailure::from(stopped)
                .connect_code()
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
pub(crate) mod tests {

    use tempfile::TempDir;

    use releash_lib::test_support::integration::workflow::resolve_workflow_by_name;
    use releash_lib::test_support::integration::workflow::CommandSpec;
    use releash_lib::test_support::integration::workflow::NodeDefinition;
    use releash_lib::test_support::integration::workflow::NodeKind;
    use releash_lib::test_support::integration::workflow::WorkflowDefinition as WorkflowDefinitionYaml;

    fn workflow(name: &str) -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: name.to_string(),
            description: "test workflow".to_string(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                    env: Default::default(),
                }),
                ..NodeDefinition::default()
            }],
            ..WorkflowDefinitionYaml::default()
        }
    }

    #[test]
    pub fn resolves_definition_name_when_filename_differs() {
        let tmp = TempDir::new().unwrap();
        releash_lib::test_support::integration::workflow::save_workflow(
            tmp.path(),
            &workflow("declared-name"),
        )
        .unwrap();
        std::fs::rename(
            tmp.path().join("declared-name.yml"),
            tmp.path().join("different-filename.yml"),
        )
        .unwrap();

        let resolved = resolve_workflow_by_name(tmp.path(), tmp.path(), "declared-name").unwrap();

        assert_eq!(resolved.name, "declared-name");
    }

    #[test]
    pub fn duplicate_definition_names_are_reported_as_diagnostic() {
        let tmp = TempDir::new().unwrap();
        releash_lib::test_support::integration::workflow::save_workflow(
            tmp.path(),
            &workflow("duplicate-name"),
        )
        .unwrap();
        std::fs::copy(
            tmp.path().join("duplicate-name.yml"),
            tmp.path().join("second-file.yml"),
        )
        .unwrap();

        let error = resolve_workflow_by_name(tmp.path(), tmp.path(), "duplicate-name")
            .expect_err("duplicate names must not be selected by directory order");

        assert!(error.to_string().contains("WFS006"));
        assert!(error.to_string().contains("duplicate-name"));
    }

    #[test]
    pub fn resolves_lua_definition_by_declared_name() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("lua-runtime.lua"),
            r#"
local r = require("releash")
return r.workflow{
  name = "lua-runtime", description = "Lua runtime",
  main = r.command{ command = "true" },
}
"#,
        )
        .unwrap();

        let resolved = resolve_workflow_by_name(tmp.path(), tmp.path(), "lua-runtime").unwrap();

        assert_eq!(resolved.name, "lua-runtime");
        assert_eq!(resolved.entry, "main");
    }

    #[test]
    pub fn duplicate_name_across_yaml_and_lua_is_reported() {
        let tmp = TempDir::new().unwrap();
        releash_lib::test_support::integration::workflow::save_workflow(
            tmp.path(),
            &workflow("duplicate-cross-format"),
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("duplicate-cross-format.lua"),
            r#"
local r = require("releash")
return r.workflow{
  name = "duplicate-cross-format", description = "Lua duplicate",
  main = r.command{ command = "true" },
}
"#,
        )
        .unwrap();

        let error =
            resolve_workflow_by_name(tmp.path(), tmp.path(), "duplicate-cross-format").unwrap_err();

        assert!(error.to_string().contains("WFS006"));
    }
}
