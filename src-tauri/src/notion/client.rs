use std::collections::HashSet;

use super::types::{
    NotionConfigStatus, NotionError, NotionLabelOption, NotionPropertyInfo, NotionRepoConfig,
    NotionTask, NotionTaskPage, NotionTaskQuery, NotionValidationResult, PropertyMapping,
};

const NOTION_API_VERSION: &str = "2025-09-03";
const NOTION_BASE_URL: &str = "https://api.notion.com/v1";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_RETRIES: u32 = 2;

fn build_client(api_token: &str) -> Result<reqwest::blocking::Client, NotionError> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_token}"))
            .map_err(|e| NotionError::RequestFailed(e.to_string()))?,
    );
    headers.insert(
        "Notion-Version",
        HeaderValue::from_static(NOTION_API_VERSION),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    reqwest::blocking::Client::builder()
        .default_headers(headers)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| NotionError::RequestFailed(e.to_string()))
}

fn send_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<reqwest::blocking::Response, NotionError> {
    let mut retries = 0;
    loop {
        let resp = client
            .post(url)
            .json(body)
            .send()
            .map_err(|e| NotionError::RequestFailed(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            if retries >= MAX_RETRIES {
                return Err(NotionError::ApiError(
                    "Rate limited after retries".to_string(),
                ));
            }
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1)
                .min(60);
            std::thread::sleep(std::time::Duration::from_secs(retry_after));
            retries += 1;
            continue;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(NotionError::ApiError(format!("HTTP {status}: {body}")));
        }

        return Ok(resp);
    }
}

fn build_notion_filter(
    query: &NotionTaskQuery,
    mapping: &PropertyMapping,
) -> Option<serde_json::Value> {
    let mut conditions = Vec::new();

    if !query.title_filter.is_empty() {
        conditions.push(serde_json::json!({
            "property": mapping.title,
            "title": { "contains": query.title_filter }
        }));
    }

    for (prop_name, value) in &query.label_filters {
        if value.is_empty() {
            continue;
        }

        let prop_type = mapping
            .labels
            .iter()
            .find(|lp| lp.name == *prop_name)
            .map(|lp| lp.property_type.as_str())
            .unwrap_or("select");

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
            // select and fallback
            _ => serde_json::json!({
                "property": prop_name,
                "select": { "equals": value }
            }),
        };
        conditions.push(filter);
    }

    match conditions.len() {
        0 => None,
        1 => Some(conditions.into_iter().next().unwrap()),
        _ => Some(serde_json::json!({ "and": conditions })),
    }
}

pub fn query_tasks(
    config: &NotionRepoConfig,
    query: &NotionTaskQuery,
) -> Result<NotionTaskPage, NotionError> {
    let client = build_client(&config.api_token)?;
    let url = format!("{NOTION_BASE_URL}/databases/{}/query", config.database_id);

    let page_size = query.page_size.unwrap_or(20);
    let mut body = serde_json::json!({ "page_size": page_size });

    if let Some(ref cursor) = query.cursor {
        body["start_cursor"] = serde_json::Value::String(cursor.clone());
    }

    if let Some(filter) = build_notion_filter(query, &config.property_mapping) {
        body["filter"] = filter;
    }

    let resp = send_with_retry(&client, &url, &body)?;

    let json: serde_json::Value = resp
        .json()
        .map_err(|e| NotionError::ParseError(e.to_string()))?;

    let tasks = parse_query_response(&json, &config.property_mapping)?;

    let has_more = json
        .get("has_more")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let next_cursor = json
        .get("next_cursor")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(NotionTaskPage {
        tasks,
        has_more,
        next_cursor,
    })
}

pub fn validate_config(config: &NotionRepoConfig) -> NotionValidationResult {
    let client = match build_client(&config.api_token) {
        Ok(c) => c,
        Err(_) => {
            return NotionValidationResult {
                status: NotionConfigStatus::InvalidToken,
                properties: vec![],
            };
        }
    };

    let url = format!("{NOTION_BASE_URL}/databases/{}", config.database_id);

    let resp = match client.get(&url).send() {
        Ok(r) => r,
        Err(_) => {
            return NotionValidationResult {
                status: NotionConfigStatus::NetworkError,
                properties: vec![],
            };
        }
    };

    let status_code = resp.status();
    if status_code == reqwest::StatusCode::UNAUTHORIZED {
        return NotionValidationResult {
            status: NotionConfigStatus::InvalidToken,
            properties: vec![],
        };
    }

    if status_code == reqwest::StatusCode::NOT_FOUND
        || status_code == reqwest::StatusCode::BAD_REQUEST
    {
        return NotionValidationResult {
            status: NotionConfigStatus::InvalidDatabase,
            properties: vec![],
        };
    }

    if !status_code.is_success() {
        return NotionValidationResult {
            status: NotionConfigStatus::InvalidDatabase,
            properties: vec![],
        };
    }

    let json: serde_json::Value = match resp.json() {
        Ok(j) => j,
        Err(_) => {
            return NotionValidationResult {
                status: NotionConfigStatus::InvalidDatabase,
                properties: vec![],
            };
        }
    };

    let properties = extract_database_properties(&json);

    NotionValidationResult {
        status: NotionConfigStatus::Configured,
        properties,
    }
}

pub fn fetch_label_options(
    config: &NotionRepoConfig,
) -> Result<Vec<NotionLabelOption>, NotionError> {
    let client = build_client(&config.api_token)?;
    let url = format!("{NOTION_BASE_URL}/databases/{}", config.database_id);

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| NotionError::RequestFailed(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(NotionError::ApiError(format!("HTTP {status}: {body}")));
    }

    let json: serde_json::Value = resp
        .json()
        .map_err(|e| NotionError::ParseError(e.to_string()))?;

    let props = extract_database_properties(&json);

    let label_names: HashSet<&str> = config
        .property_mapping
        .labels
        .iter()
        .map(|lp| lp.name.as_str())
        .collect();

    Ok(props
        .into_iter()
        .filter(|p| label_names.contains(p.name.as_str()))
        .map(|p| NotionLabelOption {
            property_name: p.name,
            property_type: p.property_type,
            options: p.options,
        })
        .collect())
}

fn extract_database_properties(db_json: &serde_json::Value) -> Vec<NotionPropertyInfo> {
    let Some(props) = db_json.get("properties").and_then(|p| p.as_object()) else {
        return vec![];
    };

    props
        .iter()
        .map(|(name, value)| {
            let property_type = value
                .get("type")
                .and_then(|t| t.as_str())
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
            .and_then(|s| s.get("options"))
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

fn parse_query_response(
    json: &serde_json::Value,
    mapping: &PropertyMapping,
) -> Result<Vec<NotionTask>, NotionError> {
    let results = json
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or_else(|| NotionError::ParseError("results フィールドがありません".to_string()))?;

    let mut tasks = Vec::with_capacity(results.len());

    for page in results {
        let id = page
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let url = page
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let created_at = page
            .get("created_time")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let last_edited_at = page
            .get("last_edited_time")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let properties = page.get("properties");

        let title = properties
            .and_then(|p| p.get(&mapping.title))
            .map(extract_property_value)
            .unwrap_or_default();

        let mut labels = std::collections::HashMap::new();
        for label_prop in &mapping.labels {
            if let Some(prop_value) = properties.and_then(|p| p.get(&label_prop.name)) {
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
                .and_then(|p| p.get(&mapping.branch_name))
                .map(extract_property_value)
                .unwrap_or_default()
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

pub fn extract_property_value(prop: &serde_json::Value) -> String {
    let prop_type = prop.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match prop_type {
        "title" => prop
            .get("title")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|rt| rt.get("plain_text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),

        "rich_text" => prop
            .get("rich_text")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|rt| rt.get("plain_text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),

        "select" => prop
            .get("select")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string(),

        "status" => prop
            .get("status")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string(),

        "multi_select" => prop
            .get("multi_select")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),

        "number" => prop
            .get("number")
            .and_then(|n| n.as_f64())
            .map(|n| n.to_string())
            .unwrap_or_default(),

        "checkbox" => prop
            .get("checkbox")
            .and_then(|b| b.as_bool())
            .map(|b| b.to_string())
            .unwrap_or_default(),

        "formula" => {
            if let Some(formula) = prop.get("formula") {
                let formula_type = formula.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match formula_type {
                    "string" => formula
                        .get("string")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    "number" => formula
                        .get("number")
                        .and_then(|n| n.as_f64())
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    "boolean" => formula
                        .get("boolean")
                        .and_then(|b| b.as_bool())
                        .map(|b| b.to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            } else {
                String::new()
            }
        }

        "unique_id" => {
            if let Some(uid) = prop.get("unique_id") {
                let prefix = uid.get("prefix").and_then(|p| p.as_str()).unwrap_or("");
                let number = uid
                    .get("number")
                    .and_then(|n| n.as_u64())
                    .map(|n| n.to_string())
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
            .and_then(|u| u.as_str())
            .unwrap_or_default()
            .to_string(),

        _ => String::new(),
    }
}

fn extract_multi_values(prop: &serde_json::Value) -> Vec<String> {
    let prop_type = prop.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match prop_type {
        "multi_select" => prop
            .get("multi_select")
            .and_then(|arr| arr.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),

        _ => {
            let val = extract_property_value(prop);
            if val.is_empty() {
                vec![]
            } else {
                vec![val]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notion::types::LabelProperty;

    #[test]
    fn extract_title_property() {
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
    fn extract_rich_text_property() {
        let prop = serde_json::json!({
            "type": "rich_text",
            "rich_text": [
                { "plain_text": "Some text" }
            ]
        });
        assert_eq!(extract_property_value(&prop), "Some text");
    }

    #[test]
    fn extract_select_property() {
        let prop = serde_json::json!({
            "type": "select",
            "select": { "name": "In Progress" }
        });
        assert_eq!(extract_property_value(&prop), "In Progress");
    }

    #[test]
    fn extract_status_property() {
        let prop = serde_json::json!({
            "type": "status",
            "status": { "name": "Done" }
        });
        assert_eq!(extract_property_value(&prop), "Done");
    }

    #[test]
    fn extract_multi_select_property() {
        let prop = serde_json::json!({
            "type": "multi_select",
            "multi_select": [
                { "name": "bug" },
                { "name": "frontend" }
            ]
        });
        assert_eq!(extract_property_value(&prop), "bug, frontend");
    }

    #[test]
    fn extract_multi_values_multi_select() {
        let prop = serde_json::json!({
            "type": "multi_select",
            "multi_select": [
                { "name": "bug" },
                { "name": "frontend" }
            ]
        });
        let values = extract_multi_values(&prop);
        assert_eq!(values, vec!["bug", "frontend"]);
    }

    #[test]
    fn extract_multi_values_fallback_to_single() {
        let prop = serde_json::json!({
            "type": "select",
            "select": { "name": "urgent" }
        });
        let values = extract_multi_values(&prop);
        assert_eq!(values, vec!["urgent"]);
    }

    #[test]
    fn extract_multi_values_empty() {
        let prop = serde_json::json!({
            "type": "select",
            "select": null
        });
        let values = extract_multi_values(&prop);
        assert!(values.is_empty());
    }

    #[test]
    fn extract_number_property() {
        let prop = serde_json::json!({
            "type": "number",
            "number": 42.0
        });
        assert_eq!(extract_property_value(&prop), "42");
    }

    #[test]
    fn extract_checkbox_property() {
        let prop = serde_json::json!({
            "type": "checkbox",
            "checkbox": true
        });
        assert_eq!(extract_property_value(&prop), "true");
    }

    #[test]
    fn extract_formula_string_property() {
        let prop = serde_json::json!({
            "type": "formula",
            "formula": { "type": "string", "string": "computed" }
        });
        assert_eq!(extract_property_value(&prop), "computed");
    }

    #[test]
    fn extract_formula_number_property() {
        let prop = serde_json::json!({
            "type": "formula",
            "formula": { "type": "number", "number": 99.0 }
        });
        assert_eq!(extract_property_value(&prop), "99");
    }

    #[test]
    fn extract_unique_id_with_prefix() {
        let prop = serde_json::json!({
            "type": "unique_id",
            "unique_id": { "prefix": "PROJ", "number": 123 }
        });
        assert_eq!(extract_property_value(&prop), "PROJ-123");
    }

    #[test]
    fn extract_unique_id_without_prefix() {
        let prop = serde_json::json!({
            "type": "unique_id",
            "unique_id": { "prefix": null, "number": 42 }
        });
        assert_eq!(extract_property_value(&prop), "42");
    }

    #[test]
    fn extract_url_property() {
        let prop = serde_json::json!({
            "type": "url",
            "url": "https://example.com"
        });
        assert_eq!(extract_property_value(&prop), "https://example.com");
    }

    #[test]
    fn extract_url_null() {
        let prop = serde_json::json!({
            "type": "url",
            "url": null
        });
        assert_eq!(extract_property_value(&prop), "");
    }

    #[test]
    fn extract_unknown_property_type() {
        let prop = serde_json::json!({
            "type": "unknown_type",
            "unknown_type": "value"
        });
        assert_eq!(extract_property_value(&prop), "");
    }

    #[test]
    fn extract_title_empty_array() {
        let prop = serde_json::json!({
            "type": "title",
            "title": []
        });
        assert_eq!(extract_property_value(&prop), "");
    }

    #[test]
    fn extract_select_null() {
        let prop = serde_json::json!({
            "type": "select",
            "select": null
        });
        assert_eq!(extract_property_value(&prop), "");
    }

    #[test]
    fn parse_query_response_basic() {
        let json = serde_json::json!({
            "results": [
                {
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
                }
            ]
        });

        let mapping = PropertyMapping::default();
        let tasks = parse_query_response(&json, &mapping).unwrap();

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "page-1");
        assert_eq!(tasks[0].title, "Task 1");
        assert!(tasks[0].labels.is_empty()); // no label properties mapped
    }

    #[test]
    fn parse_query_response_with_labels_and_branch() {
        let json = serde_json::json!({
            "results": [
                {
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
                            "multi_select": [
                                { "name": "bug" },
                                { "name": "urgent" }
                            ]
                        },
                        "Branch": {
                            "type": "rich_text",
                            "rich_text": [{ "plain_text": "fix/bug-123" }]
                        }
                    }
                }
            ]
        });

        let mapping = PropertyMapping {
            title: "Task".to_string(),
            labels: vec![
                LabelProperty {
                    name: "State".to_string(),
                    property_type: "status".to_string(),
                },
                LabelProperty {
                    name: "Tags".to_string(),
                    property_type: "multi_select".to_string(),
                },
            ],
            branch_name: "Branch".to_string(),
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
        assert_eq!(tasks[0].branch_name, "fix/bug-123");
    }

    #[test]
    fn parse_query_response_no_results() {
        let json = serde_json::json!({ "other": "data" });
        let mapping = PropertyMapping::default();
        let result = parse_query_response(&json, &mapping);
        assert!(result.is_err());
    }

    #[test]
    fn extract_database_properties_basic() {
        let db_json = serde_json::json!({
            "properties": {
                "Name": { "type": "title" },
                "Status": { "type": "status" },
                "Tags": { "type": "multi_select" },
                "Branch": { "type": "rich_text" }
            }
        });

        let props = extract_database_properties(&db_json);
        assert_eq!(props.len(), 4);

        let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Name"));
        assert!(names.contains(&"Status"));
        assert!(names.contains(&"Tags"));
        assert!(names.contains(&"Branch"));
    }

    #[test]
    fn extract_database_properties_empty() {
        let db_json = serde_json::json!({});
        let props = extract_database_properties(&db_json);
        assert!(props.is_empty());
    }

    #[test]
    fn extract_database_properties_with_options() {
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

        let props = extract_database_properties(&db_json);

        let status = props.iter().find(|p| p.name == "Status").unwrap();
        assert_eq!(status.options, vec!["Todo", "In Progress", "Done"]);

        let tags = props.iter().find(|p| p.name == "Tags").unwrap();
        assert_eq!(tags.options, vec!["frontend", "backend"]);

        let priority = props.iter().find(|p| p.name == "Priority").unwrap();
        assert_eq!(priority.options, vec!["High", "Low"]);

        let name = props.iter().find(|p| p.name == "Name").unwrap();
        assert!(name.options.is_empty());
    }

    #[test]
    fn build_notion_filter_empty_query() {
        let query = NotionTaskQuery {
            title_filter: String::new(),
            label_filters: std::collections::HashMap::new(),
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping::default();
        assert!(build_notion_filter(&query, &mapping).is_none());
    }

    #[test]
    fn build_notion_filter_title_only() {
        let query = NotionTaskQuery {
            title_filter: "検索語".to_string(),
            label_filters: std::collections::HashMap::new(),
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping {
            title: "Name".to_string(),
            labels: vec![],
            branch_name: String::new(),
        };

        let filter = build_notion_filter(&query, &mapping).unwrap();
        assert_eq!(filter["property"], "Name");
        assert_eq!(filter["title"]["contains"], "検索語");
    }

    #[test]
    fn build_notion_filter_label_only() {
        let mut label_filters = std::collections::HashMap::new();
        label_filters.insert("Status".to_string(), "Todo".to_string());

        let query = NotionTaskQuery {
            title_filter: String::new(),
            label_filters,
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping {
            title: "Name".to_string(),
            labels: vec![LabelProperty {
                name: "Status".to_string(),
                property_type: "status".to_string(),
            }],
            branch_name: String::new(),
        };

        let filter = build_notion_filter(&query, &mapping).unwrap();
        assert_eq!(filter["property"], "Status");
        assert_eq!(filter["status"]["equals"], "Todo");
    }

    #[test]
    fn build_notion_filter_multi_select() {
        let mut label_filters = std::collections::HashMap::new();
        label_filters.insert("Tags".to_string(), "frontend".to_string());

        let query = NotionTaskQuery {
            title_filter: String::new(),
            label_filters,
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping {
            title: "Name".to_string(),
            labels: vec![LabelProperty {
                name: "Tags".to_string(),
                property_type: "multi_select".to_string(),
            }],
            branch_name: String::new(),
        };

        let filter = build_notion_filter(&query, &mapping).unwrap();
        assert_eq!(filter["property"], "Tags");
        assert_eq!(filter["multi_select"]["contains"], "frontend");
    }

    #[test]
    fn build_notion_filter_multiple_conditions_and() {
        let mut label_filters = std::collections::HashMap::new();
        label_filters.insert("Status".to_string(), "Todo".to_string());

        let query = NotionTaskQuery {
            title_filter: "検索".to_string(),
            label_filters,
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping {
            title: "Name".to_string(),
            labels: vec![LabelProperty {
                name: "Status".to_string(),
                property_type: "status".to_string(),
            }],
            branch_name: String::new(),
        };

        let filter = build_notion_filter(&query, &mapping).unwrap();
        let and_conditions = filter["and"].as_array().unwrap();
        assert_eq!(and_conditions.len(), 2);
    }

    #[test]
    fn build_notion_filter_skips_empty_label_value() {
        let mut label_filters = std::collections::HashMap::new();
        label_filters.insert("Status".to_string(), String::new());

        let query = NotionTaskQuery {
            title_filter: String::new(),
            label_filters,
            cursor: None,
            page_size: None,
        };
        let mapping = PropertyMapping::default();
        assert!(build_notion_filter(&query, &mapping).is_none());
    }
}
