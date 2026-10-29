use super::*;
use crate::adaptor::gateway::app_config::config_models::{
    AgentsSection, AppSection, NotionLabelPropertyModel, NotionPropertyMappingModel,
    NotionRepoConfigModel, WorkflowSection,
};

#[test]
fn test_workflow設定の読み取り_設定モデルから出力値を返す() {
    for enabled in [false, true] {
        // Given
        let mut config = ReleashConfig::default();
        config.workflow.approval_auto_approve = enabled;
        let gateway = AppConfig::new(config, PathBuf::new());

        // When
        let config = AppConfigRepository::load(&gateway).unwrap();

        // Then
        assert_eq!(config.workflow().approval_auto_approve, enabled);
    }
}

#[test]
fn test_notion設定_保存モデルの全項目を購読用出力へ写す() {
    // Given
    let mut config = ReleashConfig::default();
    config.notion.insert(
        "/repo".into(),
        NotionRepoConfigModel {
            api_token: "ntn_token".into(),
            database_id: "db-1".into(),
            property_mapping: NotionPropertyMappingModel {
                title: "Task".into(),
                labels: vec![
                    NotionLabelPropertyModel {
                        name: "Priority".into(),
                        property_type: "select".into(),
                    },
                    NotionLabelPropertyModel {
                        name: "Tags".into(),
                        property_type: "multi_select".into(),
                    },
                ],
                branch_name: "Branch".into(),
                branch_prefix: "issue/".into(),
            },
        },
    );
    let gateway = AppConfig::new(config, std::path::PathBuf::from("unused.toml"));

    // When
    let output = crate::usecase::notion::query_service::NotionConfigQueryService::get_config(
        &gateway, "/repo",
    )
    .unwrap()
    .unwrap();

    // Then
    assert_eq!(output.api_token, "ntn_token");
    assert_eq!(output.database_id, "db-1");
    assert_eq!(output.property_mapping.title, "Task");
    assert_eq!(output.property_mapping.labels.len(), 2);
    assert_eq!(output.property_mapping.labels[0].name, "Priority");
    assert_eq!(output.property_mapping.labels[0].property_type, "select");
    assert_eq!(output.property_mapping.labels[1].name, "Tags");
    assert_eq!(
        output.property_mapping.labels[1].property_type,
        "multi_select"
    );
    assert_eq!(output.property_mapping.branch_name, "Branch");
    assert_eq!(output.property_mapping.branch_prefix, "issue/");
    assert!(!format!("{output:?}").contains("ntn_token"));
}

#[test]
fn test_notion設定_保存値が無いとき購読用出力を返さない() {
    // Given
    let gateway = AppConfig::new(ReleashConfig::default(), std::path::PathBuf::new());

    // When
    let output = crate::usecase::notion::query_service::NotionConfigQueryService::get_config(
        &gateway, "/repo",
    )
    .unwrap();

    // Then
    assert!(output.is_none());
}

#[test]
fn telemetry_defaults_to_enabled() {
    let config = ReleashConfig::default();
    assert!(config.telemetry.crash_reporting);
    assert!(config.telemetry.performance_telemetry);
}

#[test]
fn workflow_section_defaults_to_manual_approval() {
    let workflow = WorkflowSection::default();
    assert!(!workflow.approval_auto_approve);
}

#[test]
fn app_section_defaults() {
    let app = AppSection::default();
    assert!(!app.auto_launch);
    assert!(!app.start_minimized);
    assert!(app.last_repo_paths.is_empty());
    assert!(app.external_editor.is_empty());
}

#[test]
fn notion_config_model_domain_conversion_roundtrip_preserves_mapping() {
    let model = NotionRepoConfigModel {
        api_token: "ntn_test_token".to_string(),
        database_id: "db-id-456".to_string(),
        property_mapping: NotionPropertyMappingModel {
            title: "Task Name".to_string(),
            labels: vec![
                NotionLabelPropertyModel {
                    name: "Status".to_string(),
                    property_type: "status".to_string(),
                },
                NotionLabelPropertyModel {
                    name: "Tags".to_string(),
                    property_type: "multi_select".to_string(),
                },
            ],
            branch_name: "Branch".to_string(),
            branch_prefix: "feat/".to_string(),
        },
    };

    let domain = notion_to_domain(model.clone());
    let model_again = notion_to_model(domain.clone());

    assert_eq!(model_again.api_token, model.api_token);
    assert_eq!(model_again.database_id, model.database_id);
    assert_eq!(
        model_again.property_mapping.title,
        model.property_mapping.title
    );
    assert_eq!(
        model_again.property_mapping.labels,
        model.property_mapping.labels
    );
    assert_eq!(
        model_again.property_mapping.branch_name,
        model.property_mapping.branch_name
    );
    assert_eq!(
        model_again.property_mapping.branch_prefix,
        model.property_mapping.branch_prefix
    );
    assert_eq!(notion_to_domain(model_again), domain);
}

#[test]
fn agents_section_defaults() {
    let agents = AgentsSection::default();
    assert!(agents.claude.cli_path.is_none());
    assert!(agents.codex.cli_path.is_none());
}

#[test]
fn test_設定読み込み_ロック破損を技術的な失敗として返す() {
    // Given
    let _crash_guard = crate::test_support::lock_crash_telemetry();
    let gateway = AppConfig::new(ReleashConfig::default(), PathBuf::new());
    let _ = std::panic::catch_unwind(|| {
        let _lock = gateway.config.lock().unwrap();
        panic!("poison config lock");
    });

    // When
    let result = AppConfigRepository::load(&gateway);

    // Then
    assert!(matches!(result, Err(AppConfigError::Technical(failure))
        if failure.nature == TechnicalFailureNature::Other && failure.message.contains("ロック取得失敗")));
}

#[test]
fn test_terminal起動設定の読み取り_保存モデルから値を返す() {
    for command in ["", "printf '準備完了\\n'"] {
        // Given
        let mut config = ReleashConfig::default();
        config.app.terminal_startup_command = command.into();
        let gateway = AppConfig::new(config, PathBuf::new());

        // When
        let config = AppConfigRepository::load(&gateway).unwrap();

        // Then
        assert_eq!(config.terminal_startup_command(), command);
    }
}

#[test]
fn test_設定読み込み_不正なprovider実行ファイルを入力エラーとして返す() {
    for path in ["", " \t ", "invalid\0path"] {
        // Given
        let mut config = ReleashConfig::default();
        config.agents.codex.cli_path = Some(path.into());
        let gateway = AppConfig::new(config, PathBuf::new());

        // When
        let result = AppConfigRepository::load(&gateway);

        // Then
        assert!(matches!(result, Err(AppConfigError::InvalidInput(_))));
    }
}

#[test]
fn test_repository設定_ロック破損を読み込みと保存の失敗として返す() {
    // Given
    let _crash_guard = crate::test_support::lock_crash_telemetry();
    let gateway = std::sync::Arc::new(AppConfig::new(ReleashConfig::default(), PathBuf::new()));
    let poisoned = gateway.clone();
    let _ = std::thread::spawn(move || {
        let _lock = poisoned.config.lock().unwrap();
        panic!("poison config lock");
    })
    .join();

    // When / Then
    assert!(matches!(
        RepositoryConfigRepository::load(&*gateway, "/repo"),
        Err(AppConfigError::Technical(_))
    ));
    let config = crate::domain::app_config::RepositoryConfig::new("/repo".into(), None, None);
    assert!(matches!(
        RepositoryConfigRepository::save(&*gateway, config),
        Err(AppConfigError::Technical(_))
    ));
}

#[test]
fn test_設定変更_変更処理失敗時はmemoryを進めない() {
    // Given
    let gateway = AppConfig::new(ReleashConfig::default(), PathBuf::new());

    // When
    let result: Result<(), String> = gateway.with_config_mut(|config| {
        config.app.auto_launch = true;
        Err("変更失敗".into())
    });

    // Then
    assert_eq!(result, Err("変更失敗".into()));
    assert!(!gateway.get_config().unwrap().app.auto_launch);
}

#[test]
fn test_desktop設定の読み取り_保存モデルの値をentityに写す() {
    for enabled in [false, true] {
        // Given
        let mut config = ReleashConfig::default();
        config.app.start_minimized = enabled;
        config.app.auto_launch = !enabled;
        config.telemetry.crash_reporting = enabled;
        config.telemetry.performance_telemetry = !enabled;
        let gateway = AppConfig::new(config, PathBuf::new());

        // When
        let config = AppConfigRepository::load(&gateway).unwrap();

        // Then
        assert_eq!(config.start_minimized(), enabled);
        assert_eq!(config.auto_launch(), !enabled);
        assert_eq!(config.telemetry().crash_reporting, enabled);
        assert_eq!(config.telemetry().performance_telemetry, !enabled);
    }
}
