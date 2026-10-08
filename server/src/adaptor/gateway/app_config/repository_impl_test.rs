use super::*;
use crate::adaptor::gateway::app_config::config_models::{
    AgentsSection, AppSection, WorkflowSection,
};

#[test]
fn test_workflow設定の読み取り_設定モデルから出力値を返す() {
    // Given
    let mut config = ReleashConfig::default();
    config.workflow.approval_auto_approve = true;
    let gateway = AppConfig::new(config, std::path::PathBuf::new());

    // When
    let output = gateway.get_workflow_config().unwrap();
    // Then
    assert!(output.approval_auto_approve);
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
    assert!(app.close_to_tray);
    assert!(!app.auto_launch);
    assert!(!app.start_minimized);
    assert!(app.last_root_path.is_empty());
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
fn test_workflow設定の読み取り_ロック破損を失敗として返す() {
    // Given
    let gateway = std::sync::Arc::new(AppConfig::new(
        ReleashConfig::default(),
        std::path::PathBuf::new(),
    ));
    let poisoned = gateway.clone();
    let _ = std::thread::spawn(move || {
        let _lock = poisoned.config.lock().unwrap();
        panic!("poison config lock");
    })
    .join();

    // When
    let result = gateway.get_workflow_config();

    // Then
    assert!(matches!(result, Err(AppConfigError::Repository(_))));
}
