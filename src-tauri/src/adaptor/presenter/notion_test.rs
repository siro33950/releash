pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_notion_status_view_snake_caseでserializeされる() {
        let status = serde_json::to_string(&NotionConfigStatusView::InvalidToken).unwrap();
        assert_eq!(status, r#""invalid_token""#);

        let status = serde_json::to_string(&NotionConfigStatusView::NotConfigured).unwrap();
        assert_eq!(status, r#""not_configured""#);
    }

    #[test]
    fn test_notion_repo_config_view_debugでtokenをマスクする() {
        let config = NotionRepoConfigView {
            api_token: "ntn_secret_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: PropertyMappingView::default(),
        };

        let output = format!("{config:?}");
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("ntn_secret_token"));
    }

    #[test]
    fn test_property_mapping_view_省略値を維持する() {
        let mapping: PropertyMappingView = toml::from_str("").unwrap();

        assert_eq!(mapping.title, "Name");
        assert!(mapping.labels.is_empty());
        assert!(mapping.branch_name.is_empty());
        assert!(mapping.branch_prefix.is_empty());
    }

    #[test]
    fn test_property_mapping_view_構造化labels配列を読む() {
        let json = r#"{
            "title": "Task",
            "labels": [
                { "name": "Status", "property_type": "status" },
                { "name": "Tags", "property_type": "multi_select" }
            ],
            "branch_name": "Branch",
            "branch_prefix": "feat/"
        }"#;

        let mapping: PropertyMappingView = serde_json::from_str(json).unwrap();

        assert_eq!(mapping.title, "Task");
        assert_eq!(mapping.labels.len(), 2);
        assert_eq!(mapping.labels[0].name, "Status");
        assert_eq!(mapping.labels[0].property_type, "status");
        assert_eq!(mapping.labels[1].name, "Tags");
        assert_eq!(mapping.labels[1].property_type, "multi_select");
        assert_eq!(mapping.branch_name, "Branch");
        assert_eq!(mapping.branch_prefix, "feat/");
    }

    #[test]
    fn test_property_mapping_view_labels文字列配列は受け付けない() {
        let json = r#"{ "labels": ["Status", "Tags"] }"#;

        assert!(serde_json::from_str::<PropertyMappingView>(json).is_err());
    }

    #[test]
    fn test_notion_label_option_view_option_ids省略時は空になる() {
        let json = r#"{
            "property_name": "Status",
            "property_type": "status",
            "options": ["Todo", "Done"]
        }"#;
        let deserialized: NotionLabelOptionView = serde_json::from_str(json).unwrap();

        assert_eq!(deserialized.property_name, "Status");
        assert!(deserialized.option_ids.is_empty());
    }
}
