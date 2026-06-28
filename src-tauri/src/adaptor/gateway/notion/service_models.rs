use crate::domain::app_config::value_objects::NotionPropertyMapping;
use crate::domain::notion::services::notion_task_title_branch_name;
use crate::domain::notion::{NotionError, NotionPropertyInfo, NotionTask, NotionTaskQuery};

pub(crate) fn build_notion_filter(
    query: &NotionTaskQuery,
    mapping: &NotionPropertyMapping,
) -> Option<serde_json::Value> {
    let mut conditions = Vec::new();

    if !query.title_filter.is_empty() {
        conditions.push(serde_json::json!({
            "property": mapping.title,
            "title": { "contains": query.title_filter }
        }));
    }

    for (prop_name, values) in &query.label_filters {
        let values: Vec<&str> = values
            .iter()
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .collect();
        if values.is_empty() {
            continue;
        }

        let prop_type = mapping
            .labels
            .iter()
            .find(|label| label.name == *prop_name)
            .map(|label| label.property_type.as_str())
            .unwrap_or("select");

        if values.len() == 1 {
            let value = values[0];
            let filter = match prop_type {
                "multi_select" => serde_json::json!({
                    "property": prop_name,
                    "multi_select": { "contains": value }
                }),
                "status" => serde_json::json!({
                    "property": prop_name,
                    "status": { "equals": value }
                }),
                "rich_text" => serde_json::json!({
                    "property": prop_name,
                    "rich_text": { "contains": value }
                }),
                "people" => serde_json::json!({
                    "property": prop_name,
                    "people": { "contains": value }
                }),
                _ => serde_json::json!({
                    "property": prop_name,
                    "select": { "equals": value }
                }),
            };
            conditions.push(filter);
        } else {
            match prop_type {
                "multi_select" => {
                    for value in &values {
                        conditions.push(serde_json::json!({
                            "property": prop_name,
                            "multi_select": { "contains": value }
                        }));
                    }
                }
                "select" | "status" => {
                    let or_conditions: Vec<serde_json::Value> = values
                        .iter()
                        .map(|value| {
                            serde_json::json!({
                                "property": prop_name,
                                prop_type: { "equals": value }
                            })
                        })
                        .collect();
                    conditions.push(serde_json::json!({ "or": or_conditions }));
                }
                "people" => {
                    let or_conditions: Vec<serde_json::Value> = values
                        .iter()
                        .map(|value| {
                            serde_json::json!({
                                "property": prop_name,
                                "people": { "contains": value }
                            })
                        })
                        .collect();
                    conditions.push(serde_json::json!({ "or": or_conditions }));
                }
                "rich_text" => {
                    let or_conditions: Vec<serde_json::Value> = values
                        .iter()
                        .map(|value| {
                            serde_json::json!({
                                "property": prop_name,
                                "rich_text": { "contains": value }
                            })
                        })
                        .collect();
                    conditions.push(serde_json::json!({ "or": or_conditions }));
                }
                _ => {
                    let or_conditions: Vec<serde_json::Value> = values
                        .iter()
                        .map(|value| {
                            serde_json::json!({
                                "property": prop_name,
                                "select": { "equals": value }
                            })
                        })
                        .collect();
                    conditions.push(serde_json::json!({ "or": or_conditions }));
                }
            }
        }
    }

    match conditions.len() {
        0 => None,
        1 => Some(conditions.into_iter().next().unwrap()),
        _ => Some(serde_json::json!({ "and": conditions })),
    }
}

pub(crate) fn extract_first_data_source_id(db_json: &serde_json::Value) -> Option<String> {
    db_json
        .get("data_sources")
        .and_then(|data_sources| data_sources.as_array())
        .and_then(|arr| arr.first())
        .and_then(|data_source| data_source.get("id"))
        .and_then(|id| id.as_str())
        .map(String::from)
}

pub(crate) fn extract_properties_from_json(json: &serde_json::Value) -> Vec<NotionPropertyInfo> {
    let Some(props) = json.get("properties").and_then(|props| props.as_object()) else {
        return Vec::new();
    };

    props
        .iter()
        .map(|(name, value)| {
            let property_type = value
                .get("type")
                .and_then(|property_type| property_type.as_str())
                .unwrap_or("unknown")
                .to_string();
            let options = extract_property_options(value, &property_type);
            NotionPropertyInfo {
                name: name.clone(),
                property_type,
                options,
            }
        })
        .collect()
}

fn extract_property_options(prop_schema: &serde_json::Value, property_type: &str) -> Vec<String> {
    match property_type {
        "select" | "multi_select" | "status" => prop_schema
            .get(property_type)
            .and_then(|schema| schema.get("options"))
            .and_then(|options| options.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|name| name.as_str()))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(crate) fn parse_query_response(
    json: &serde_json::Value,
    mapping: &NotionPropertyMapping,
) -> Result<Vec<NotionTask>, NotionError> {
    let results = json
        .get("results")
        .and_then(|results| results.as_array())
        .ok_or_else(|| NotionError::ParseError("results フィールドがありません".to_string()))?;

    let mut tasks = Vec::with_capacity(results.len());

    for page in results {
        let id = page
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let url = page
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let created_at = page
            .get("created_time")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let last_edited_at = page
            .get("last_edited_time")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();

        let properties = page.get("properties");
        let title = properties
            .and_then(|props| props.get(&mapping.title))
            .map(extract_property_value)
            .unwrap_or_default();

        let mut labels = std::collections::HashMap::new();
        for label_prop in &mapping.labels {
            if let Some(prop_value) = properties.and_then(|props| props.get(&label_prop.name)) {
                let values = extract_multi_values(prop_value);
                if !values.is_empty() {
                    labels.insert(label_prop.name.clone(), values);
                }
            }
        }

        let branch_name = if mapping.branch_name.is_empty() {
            String::new()
        } else {
            properties
                .and_then(|props| props.get(&mapping.branch_name))
                .map(extract_property_value)
                .unwrap_or_default()
        };
        let branch_name = if branch_name.is_empty() {
            notion_task_title_branch_name(&title)
        } else {
            branch_name
        };

        tasks.push(NotionTask {
            id,
            title,
            url,
            labels,
            branch_name,
            created_at,
            last_edited_at,
        });
    }

    Ok(tasks)
}

pub(crate) fn extract_property_value(prop: &serde_json::Value) -> String {
    let prop_type = prop
        .get("type")
        .and_then(|property_type| property_type.as_str())
        .unwrap_or("");

    match prop_type {
        "title" => prop
            .get("title")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|rich_text| {
                        rich_text.get("plain_text").and_then(|text| text.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        "rich_text" => prop
            .get("rich_text")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|rich_text| {
                        rich_text.get("plain_text").and_then(|text| text.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        "select" => prop
            .get("select")
            .and_then(|select| select.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string(),
        "status" => prop
            .get("status")
            .and_then(|status| status.get("name"))
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string(),
        "multi_select" => prop
            .get("multi_select")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|name| name.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        "number" => prop
            .get("number")
            .and_then(|number| number.as_f64())
            .map(|number| number.to_string())
            .unwrap_or_default(),
        "checkbox" => prop
            .get("checkbox")
            .and_then(|value| value.as_bool())
            .map(|value| value.to_string())
            .unwrap_or_default(),
        "formula" => {
            if let Some(formula) = prop.get("formula") {
                let formula_type = formula
                    .get("type")
                    .and_then(|formula_type| formula_type.as_str())
                    .unwrap_or("");
                match formula_type {
                    "string" => formula
                        .get("string")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    "number" => formula
                        .get("number")
                        .and_then(|value| value.as_f64())
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    "boolean" => formula
                        .get("boolean")
                        .and_then(|value| value.as_bool())
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            } else {
                String::new()
            }
        }
        "unique_id" => {
            if let Some(unique_id) = prop.get("unique_id") {
                let prefix = unique_id
                    .get("prefix")
                    .and_then(|prefix| prefix.as_str())
                    .unwrap_or("");
                let number = unique_id
                    .get("number")
                    .and_then(|number| number.as_u64())
                    .map(|number| number.to_string())
                    .unwrap_or_default();
                if prefix.is_empty() {
                    number
                } else {
                    format!("{prefix}-{number}")
                }
            } else {
                String::new()
            }
        }
        "url" => prop
            .get("url")
            .and_then(|url| url.as_str())
            .unwrap_or_default()
            .to_string(),
        "people" => prop
            .get("people")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|person| person.get("name").and_then(|name| name.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn extract_multi_values(prop: &serde_json::Value) -> Vec<String> {
    let prop_type = prop
        .get("type")
        .and_then(|property_type| property_type.as_str())
        .unwrap_or("");

    match prop_type {
        "multi_select" => prop
            .get("multi_select")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|name| name.as_str()))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        "people" => prop
            .get("people")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|person| person.get("name").and_then(|name| name.as_str()))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        _ => {
            let value = extract_property_value(prop);
            if value.is_empty() {
                Vec::new()
            } else {
                vec![value]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::domain::app_config::value_objects::NotionLabelProperty;

    use super::*;

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
        assert_eq!(tasks[0].branch_name, "feat/task-1");
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
    fn test_query_response_parse_branch未設定ならtitle由来fallbackを返す() {
        let json = serde_json::json!({
            "results": [{
                "id": "page-3",
                "url": "https://notion.so/page-3",
                "created_time": "2026-01-01T00:00:00.000Z",
                "last_edited_time": "2026-01-02T00:00:00.000Z",
                "properties": {
                    "Name": {
                        "type": "title",
                        "title": [{ "plain_text": "Move Notion branch rules" }]
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

        let tasks = parse_query_response(&json, &mapping).unwrap();

        assert_eq!(tasks[0].branch_name, "feat/move-notion-branch-rules");
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
