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
#[path = "service_models_test.rs"]
mod service_models_tests;
