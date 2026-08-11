use super::*;

fn assert_domain_roundtrip(config: ReleashConfig) {
    let domain = config_to_domain(&config);
    let mut roundtripped = config.clone();

    apply_domain_to_config(&mut roundtripped, domain);

    assert_eq!(roundtripped.server, config.server);
    assert_eq!(roundtripped.telemetry, config.telemetry);
    assert_eq!(roundtripped.app, config.app);
    assert_eq!(roundtripped.workflow, config.workflow);
    assert_eq!(roundtripped.notion.len(), config.notion.len());
    assert_eq!(
        roundtripped.agents.claude.cli_path,
        config.agents.claude.cli_path
    );
    assert_eq!(
        roundtripped.agents.codex.cli_path,
        config.agents.codex.cli_path
    );
}

#[test]
fn test_config_model変換_既定値がdomain往復で同値になる() {
    assert_domain_roundtrip(ReleashConfig::default());
}

#[test]
fn test_config_model変換_変更済み値がdomain往復で同値になる() {
    let config = ReleashConfig {
        server: ServerSection {
            bind: "0.0.0.0".to_string(),
            port: 18080,
            token: "server-token".to_string(),
            tls: TlsSection {
                enabled: true,
                cert: "/tmp/cert.pem".to_string(),
                key: "/tmp/key.pem".to_string(),
            },
        },
        telemetry: TelemetrySection {
            crash_reporting: false,
            performance_telemetry: false,
        },
        app: AppSection {
            close_to_tray: false,
            auto_launch: true,
            start_minimized: true,
            last_root_path: "/repo".to_string(),
            last_repo_paths: vec!["/repo".to_string(), "/repo2".to_string()],
            external_editor: "cursor".to_string(),
        },
        agents: AgentsSection {
            claude: ClaudeAgentSection {
                cli_path: Some("/opt/bin/claude".to_string()),
            },
            codex: CodexAgentSection {
                cli_path: Some("/opt/bin/codex".to_string()),
            },
        },
        workflow: WorkflowSection {
            approval_auto_approve: true,
        },
        ..ReleashConfig::default()
    };

    assert_domain_roundtrip(config);
}

#[test]
fn test_notion_config_model_toml_roundtripする() {
    let config = NotionRepoConfigModel {
        api_token: "ntn_test_token".to_string(),
        database_id: "db-id-456".to_string(),
        property_mapping: NotionPropertyMappingModel {
            title: "Task Name".to_string(),
            labels: vec![
                NotionLabelPropertyModel {
                    name: "Tags".to_string(),
                    property_type: "multi_select".to_string(),
                },
                NotionLabelPropertyModel {
                    name: "Status".to_string(),
                    property_type: "status".to_string(),
                },
            ],
            branch_name: "Branch".to_string(),
            branch_prefix: "feat/".to_string(),
        },
    };

    let toml_str = toml::to_string_pretty(&config).unwrap();
    let deserialized: NotionRepoConfigModel = toml::from_str(&toml_str).unwrap();

    assert_eq!(deserialized.api_token, config.api_token);
    assert_eq!(deserialized.database_id, config.database_id);
    assert_eq!(deserialized.property_mapping.title, "Task Name");
    assert_eq!(deserialized.property_mapping.labels.len(), 2);
    assert_eq!(deserialized.property_mapping.labels[0].name, "Tags");
    assert_eq!(
        deserialized.property_mapping.labels[0].property_type,
        "multi_select"
    );
    assert_eq!(deserialized.property_mapping.branch_prefix, "feat/");
}

#[test]
fn test_notion_config_model_省略mapping項目は既定値になる() {
    let toml_str = r#"
api_token = "ntn_test"
database_id = "db-123"
"#;

    let config: NotionRepoConfigModel = toml::from_str(toml_str).unwrap();

    assert_eq!(config.property_mapping.title, "Name");
    assert!(config.property_mapping.labels.is_empty());
    assert!(config.property_mapping.branch_name.is_empty());
    assert!(config.property_mapping.branch_prefix.is_empty());
}

#[test]
fn test_notion_config_model_labels旧文字列配列をselectとして読む() {
    let toml_str = r#"
api_token = "ntn_test"
database_id = "db-123"

[property_mapping]
title = "Name"
labels = ["Status", "Tags"]
"#;

    let config: NotionRepoConfigModel = toml::from_str(toml_str).unwrap();

    assert_eq!(config.property_mapping.labels.len(), 2);
    assert_eq!(config.property_mapping.labels[0].name, "Status");
    assert_eq!(config.property_mapping.labels[0].property_type, "select");
    assert_eq!(config.property_mapping.labels[1].name, "Tags");
    assert_eq!(config.property_mapping.labels[1].property_type, "select");
}

#[test]
fn test_notion_config_model_debugでtokenをマスクする() {
    let config = NotionRepoConfigModel {
        api_token: "ntn_secret_token".to_string(),
        database_id: "db-1".to_string(),
        property_mapping: NotionPropertyMappingModel::default(),
    };

    let output = format!("{config:?}");

    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains("ntn_secret_token"));
}

#[test]
fn test_設定serialize_legacy_hook_portを含めない() {
    let serialized = toml::to_string_pretty(&ReleashConfig::default()).unwrap();

    assert!(!serialized.contains("hook_port"), "{serialized}");
}

#[test]
fn test_agent_tui_atomic_cutover_旧defaultとmodelsを再出力せずcli_pathを保持する() {
    let legacy = r#"
[agents]
default = "codex"

[agents.claude]
cli_path = "/opt/bin/claude"
models = ["legacy-claude"]

[agents.codex]
cli_path = "/opt/bin/codex"
models = ["legacy-codex"]
"#;
    let config: ReleashConfig = toml::from_str(legacy).unwrap();

    let serialized = toml::to_string_pretty(&config).unwrap();

    assert!(!serialized.contains("default ="), "{serialized}");
    assert!(!serialized.contains("models ="), "{serialized}");
    assert!(serialized.contains("cli_path = \"/opt/bin/claude\""));
    assert!(serialized.contains("cli_path = \"/opt/bin/codex\""));
}
