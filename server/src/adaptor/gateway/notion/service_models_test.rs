pub(crate) mod tests {
    use std::collections::HashMap;

    use crate::domain::app_config::value_objects::NotionLabelProperty;

    use super::super::*;

    fn query(
        title_filter: impl Into<String>,
        label_filters: HashMap<String, Vec<String>>,
    ) -> NotionTaskQuery {
        NotionTaskQuery {
            title_filter: title_filter.into(),
            label_filters,
            cursor: None,
            page_size: None,
        }
    }

    #[test]
    fn test_property値抽出_titleを結合する() {
        let prop = serde_json::json!({
            "type": "title",
            "title": [
                { "plain_text": "Hello " },
                { "plain_text": "World" }
            ]
        });
        assert_eq!(extract_property_value(&prop), "Hello World");
    }

    #[test]
    fn test_property値抽出_rich_textを結合する() {
        let prop = serde_json::json!({
            "type": "rich_text",
            "rich_text": [{ "plain_text": "Some text" }]
        });
        assert_eq!(extract_property_value(&prop), "Some text");
    }

    #[test]
    fn test_property値抽出_select_status_multi_select_peopleを読む() {
        let select = serde_json::json!({
            "type": "select",
            "select": { "name": "In Progress" }
        });
        let status = serde_json::json!({
            "type": "status",
            "status": { "name": "Done" }
        });
        let multi_select = serde_json::json!({
            "type": "multi_select",
            "multi_select": [{ "name": "bug" }, { "name": "frontend" }]
        });
        let people = serde_json::json!({
            "type": "people",
            "people": [
                { "object": "user", "id": "user-1", "name": "Alice" },
                { "object": "user", "id": "user-2", "name": "Bob" }
            ]
        });

        assert_eq!(extract_property_value(&select), "In Progress");
        assert_eq!(extract_property_value(&status), "Done");
        assert_eq!(extract_property_value(&multi_select), "bug, frontend");
        assert_eq!(extract_property_value(&people), "Alice, Bob");
    }

    #[test]
    fn test_property値抽出_number_checkbox_formula_unique_id_urlを読む() {
        let number = serde_json::json!({ "type": "number", "number": 42.0 });
        let checkbox = serde_json::json!({ "type": "checkbox", "checkbox": true });
        let formula_string = serde_json::json!({
            "type": "formula",
            "formula": { "type": "string", "string": "computed" }
        });
        let formula_number = serde_json::json!({
            "type": "formula",
            "formula": { "type": "number", "number": 99.0 }
        });
        let unique_id = serde_json::json!({
            "type": "unique_id",
            "unique_id": { "prefix": "PROJ", "number": 123 }
        });
        let unique_id_without_prefix = serde_json::json!({
            "type": "unique_id",
            "unique_id": { "prefix": null, "number": 42 }
        });
        let url = serde_json::json!({
            "type": "url",
            "url": "https://example.com"
        });

        assert_eq!(extract_property_value(&number), "42");
        assert_eq!(extract_property_value(&checkbox), "true");
        assert_eq!(extract_property_value(&formula_string), "computed");
        assert_eq!(extract_property_value(&formula_number), "99");
        assert_eq!(extract_property_value(&unique_id), "PROJ-123");
        assert_eq!(extract_property_value(&unique_id_without_prefix), "42");
        assert_eq!(extract_property_value(&url), "https://example.com");
    }

    #[test]
    fn test_property値抽出_nullや未知型は空文字になる() {
        let unknown = serde_json::json!({
            "type": "unknown_type",
            "unknown_type": "value"
        });
        let empty_title = serde_json::json!({ "type": "title", "title": [] });
        let null_select = serde_json::json!({ "type": "select", "select": null });
        let null_url = serde_json::json!({ "type": "url", "url": null });

        assert_eq!(extract_property_value(&unknown), "");
        assert_eq!(extract_property_value(&empty_title), "");
        assert_eq!(extract_property_value(&null_select), "");
        assert_eq!(extract_property_value(&null_url), "");
    }

    #[test]
    fn test_multi値抽出_multi_selectとpeopleは配列で返す() {
        let multi_select = serde_json::json!({
            "type": "multi_select",
            "multi_select": [{ "name": "bug" }, { "name": "frontend" }]
        });
        let people = serde_json::json!({
            "type": "people",
            "people": [
                { "object": "user", "id": "user-1", "name": "Alice" },
                { "object": "user", "id": "user-2", "name": "Bob" }
            ]
        });

        assert_eq!(extract_multi_values(&multi_select), vec!["bug", "frontend"]);
        assert_eq!(extract_multi_values(&people), vec!["Alice", "Bob"]);
    }

    #[test]
    fn test_multi値抽出_単一値へfallbackし空値は空配列になる() {
        let select = serde_json::json!({
            "type": "select",
            "select": { "name": "urgent" }
        });
        let empty = serde_json::json!({ "type": "select", "select": null });

        assert_eq!(extract_multi_values(&select), vec!["urgent"]);
        assert!(extract_multi_values(&empty).is_empty());
    }

    #[test]
    fn test_query_response_parse_basic() {
        let json = serde_json::json!({
            "results": [{
                "id": "page-1",
                "url": "https://notion.so/page-1",
                "created_time": "2026-01-01T00:00:00.000Z",
                "last_edited_time": "2026-01-02T00:00:00.000Z",
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{ "plain_text": "Task 1" }]
                    },
                    "Status": {
                        "type": "select",
                        "select": { "name": "Todo" }
                    }
                }
            }]
        });

        let tasks = parse_query_response(&json, &NotionPropertyMapping::default()).unwrap();

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "page-1");
        assert_eq!(tasks[0].title, "Task 1");
        assert_eq!(tasks[0].branch_name, "feat/page-1");
        assert!(tasks[0].labels.is_empty());
    }

    #[test]
    fn test_query_response_parse_labels_branch_peopleを読む() {
        let json = serde_json::json!({
            "results": [{
                "id": "page-2",
                "url": "https://notion.so/page-2",
                "created_time": "2026-01-01T00:00:00.000Z",
                "last_edited_time": "2026-01-02T00:00:00.000Z",
                "properties": {
                    "Task": {
                        "type": "title",
                        "title": [{ "plain_text": "Fix bug" }]
                    },
                    "State": {
                        "type": "status",
                        "status": { "name": "In Progress" }
                    },
                    "Tags": {
                        "type": "multi_select",
                        "multi_select": [{ "name": "bug" }, { "name": "urgent" }]
                    },
                    "Assignee": {
                        "type": "people",
                        "people": [
                            { "object": "user", "id": "u1", "name": "Alice" },
                            { "object": "user", "id": "u2", "name": "Bob" }
                        ]
                    },
                    "Branch": {
                        "type": "rich_text",
                        "rich_text": [{ "plain_text": "fix/bug-123" }]
                    }
                }
            }]
        });
        let mapping = NotionPropertyMapping {
            title: "Task".to_string(),
            labels: vec![
                NotionLabelProperty {
                    name: "State".to_string(),
                    property_type: "status".to_string(),
                },
                NotionLabelProperty {
                    name: "Tags".to_string(),
                    property_type: "multi_select".to_string(),
                },
                NotionLabelProperty {
                    name: "Assignee".to_string(),
                    property_type: "people".to_string(),
                },
            ],
            branch_name: "Branch".to_string(),
            branch_prefix: String::new(),
        };

        let tasks = parse_query_response(&json, &mapping).unwrap();

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Fix bug");
        assert_eq!(
            tasks[0].labels.get("State").unwrap(),
            &vec!["In Progress".to_string()]
        );
        assert_eq!(
            tasks[0].labels.get("Tags").unwrap(),
            &vec!["bug".to_string(), "urgent".to_string()]
        );
        assert_eq!(
            tasks[0].labels.get("Assignee").unwrap(),
            &vec!["Alice".to_string(), "Bob".to_string()]
        );
        assert_eq!(tasks[0].branch_name, "fix/bug-123");
    }

    #[test]
    fn test_query_response_parse_同名の日本語taskでもidごとにbranchを返す() {
        // Given
        let mut json = serde_json::json!({
            "results": [{
                "id": "page-3",
                "url": "https://notion.so/page-3",
                "created_time": "2026-01-01T00:00:00.000Z",
                "last_edited_time": "2026-01-02T00:00:00.000Z",
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{ "plain_text": "ログイン" }]
                    }
                }
            }]
        });
        let mapping = NotionPropertyMapping {
            title: "Name".to_string(),
            labels: Vec::new(),
            branch_name: "Branch".to_string(),
            branch_prefix: String::new(),
        };
        let mut second = json["results"][0].clone();
        second["id"] = serde_json::json!("page-5");
        json["results"].as_array_mut().unwrap().push(second);

        // When
        let tasks = parse_query_response(&json, &mapping).unwrap();

        // Then
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, tasks[1].title);
        assert_eq!(tasks[0].branch_name, "feat/page-3");
        assert_eq!(tasks[1].branch_name, "feat/page-5");
    }

    #[test]
    fn test_query_response_parse_branch_propertyはsanitizeやprefixを適用せず保持する() {
        let json = serde_json::json!({
            "results": [{
                "id": "page-4",
                "url": "https://notion.so/page-4",
                "created_time": "2026-01-01T00:00:00.000Z",
                "last_edited_time": "2026-01-02T00:00:00.000Z",
                "properties": {
                    "Task": {
                        "type": "title",
                        "title": [{ "plain_text": "Ignored title" }]
                    },
                    "Branch": {
                        "type": "rich_text",
                        "rich_text": [{ "plain_text": "fix login bug" }]
                    }
                }
            }]
        });
        let mapping = NotionPropertyMapping {
            title: "Task".to_string(),
            labels: Vec::new(),
            branch_name: "Branch".to_string(),
            branch_prefix: "feat/".to_string(),
        };

        let tasks = parse_query_response(&json, &mapping).unwrap();

        assert_eq!(tasks[0].branch_name, "fix login bug");
    }

    #[test]
    fn test_query_response_parse_results欠落はparse_errorになる() {
        let json = serde_json::json!({ "other": "data" });
        let result = parse_query_response(&json, &NotionPropertyMapping::default());

        assert_eq!(
            result.unwrap_err().to_string(),
            "パースエラー: results フィールドがありません"
        );
    }

    #[test]
    fn test_property一覧抽出_optionsを含める() {
        let db_json = serde_json::json!({
            "properties": {
                "Status": {
                    "type": "status",
                    "status": {
                        "options": [
                            { "name": "Todo", "color": "default" },
                            { "name": "In Progress", "color": "blue" },
                            { "name": "Done", "color": "green" }
                        ]
                    }
                },
                "Tags": {
                    "type": "multi_select",
                    "multi_select": {
                        "options": [
                            { "name": "frontend", "color": "blue" },
                            { "name": "backend", "color": "green" }
                        ]
                    }
                },
                "Priority": {
                    "type": "select",
                    "select": {
                        "options": [
                            { "name": "High", "color": "red" },
                            { "name": "Low", "color": "gray" }
                        ]
                    }
                },
                "Name": { "type": "title" }
            }
        });

        let props = extract_properties_from_json(&db_json);

        assert_eq!(props.len(), 4);
        let status = props.iter().find(|prop| prop.name == "Status").unwrap();
        assert_eq!(status.options, vec!["Todo", "In Progress", "Done"]);
        let tags = props.iter().find(|prop| prop.name == "Tags").unwrap();
        assert_eq!(tags.options, vec!["frontend", "backend"]);
        let priority = props.iter().find(|prop| prop.name == "Priority").unwrap();
        assert_eq!(priority.options, vec!["High", "Low"]);
        let name = props.iter().find(|prop| prop.name == "Name").unwrap();
        assert!(name.options.is_empty());
    }

    #[test]
    fn test_property一覧抽出_properties欠落は空配列になる() {
        let props = extract_properties_from_json(&serde_json::json!({}));
        assert!(props.is_empty());
    }

    #[test]
    fn test_filter構築_empty_queryはnone() {
        assert!(build_notion_filter(
            &query("", HashMap::new()),
            &NotionPropertyMapping::default()
        )
        .is_none());
    }

    #[test]
    fn test_filter構築_title_only() {
        let filter = build_notion_filter(
            &query("検索語", HashMap::new()),
            &NotionPropertyMapping {
                title: "Name".to_string(),
                labels: Vec::new(),
                branch_name: String::new(),
                branch_prefix: String::new(),
            },
        )
        .unwrap();

        assert_eq!(filter["property"], "Name");
        assert_eq!(filter["title"]["contains"], "検索語");
    }

    #[test]
    fn test_filter構築_label型ごとの単一値条件を作る() {
        for (property_type, filter_key, op_key) in [
            ("status", "status", "equals"),
            ("multi_select", "multi_select", "contains"),
            ("rich_text", "rich_text", "contains"),
            ("people", "people", "contains"),
            ("select", "select", "equals"),
        ] {
            let mut label_filters = HashMap::new();
            label_filters.insert("Field".to_string(), vec!["Value".to_string()]);
            let mapping = NotionPropertyMapping {
                title: "Name".to_string(),
                labels: vec![NotionLabelProperty {
                    name: "Field".to_string(),
                    property_type: property_type.to_string(),
                }],
                branch_name: String::new(),
                branch_prefix: String::new(),
            };

            let filter = build_notion_filter(&query("", label_filters), &mapping).unwrap();

            assert_eq!(filter["property"], "Field");
            assert_eq!(filter[filter_key][op_key], "Value");
        }
    }

    #[test]
    fn test_filter構築_titleとlabelはandになる() {
        let mut label_filters = HashMap::new();
        label_filters.insert("Status".to_string(), vec!["Todo".to_string()]);
        let mapping = NotionPropertyMapping {
            title: "Name".to_string(),
            labels: vec![NotionLabelProperty {
                name: "Status".to_string(),
                property_type: "status".to_string(),
            }],
            branch_name: String::new(),
            branch_prefix: String::new(),
        };

        let filter = build_notion_filter(&query("検索", label_filters), &mapping).unwrap();

        let and_conditions = filter["and"].as_array().unwrap();
        assert_eq!(and_conditions.len(), 2);
    }

    #[test]
    fn test_filter構築_空label値はskipする() {
        for values in [vec![String::new()], Vec::new()] {
            let mut label_filters = HashMap::new();
            label_filters.insert("Status".to_string(), values);

            assert!(build_notion_filter(
                &query("", label_filters),
                &NotionPropertyMapping::default()
            )
            .is_none());
        }
    }

    #[test]
    fn test_filter構築_multi_select複数値はandになる() {
        let mut label_filters = HashMap::new();
        label_filters.insert(
            "Tags".to_string(),
            vec!["frontend".to_string(), "bug".to_string()],
        );
        let mapping = NotionPropertyMapping {
            title: "Name".to_string(),
            labels: vec![NotionLabelProperty {
                name: "Tags".to_string(),
                property_type: "multi_select".to_string(),
            }],
            branch_name: String::new(),
            branch_prefix: String::new(),
        };

        let filter = build_notion_filter(&query("", label_filters), &mapping).unwrap();

        let and_conditions = filter["and"].as_array().unwrap();
        assert_eq!(and_conditions.len(), 2);
        assert_eq!(and_conditions[0]["multi_select"]["contains"], "frontend");
        assert_eq!(and_conditions[1]["multi_select"]["contains"], "bug");
    }

    #[test]
    fn test_filter構築_select_status_people複数値はorになる() {
        for (property_type, filter_key) in [
            ("select", "select"),
            ("status", "status"),
            ("people", "people"),
            ("rich_text", "rich_text"),
        ] {
            let mut label_filters = HashMap::new();
            label_filters.insert(
                "Field".to_string(),
                vec!["Value 1".to_string(), "Value 2".to_string()],
            );
            let mapping = NotionPropertyMapping {
                title: "Name".to_string(),
                labels: vec![NotionLabelProperty {
                    name: "Field".to_string(),
                    property_type: property_type.to_string(),
                }],
                branch_name: String::new(),
                branch_prefix: String::new(),
            };

            let filter = build_notion_filter(&query("", label_filters), &mapping).unwrap();

            let or_conditions = filter["or"].as_array().unwrap();
            assert_eq!(or_conditions.len(), 2);
            assert_eq!(or_conditions[0]["property"], "Field");
            assert_eq!(or_conditions[1]["property"], "Field");
            assert!(or_conditions[0].get(filter_key).is_some());
        }
    }
}
