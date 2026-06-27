use std::collections::HashSet;

use crate::domain::app_config::value_objects::{NotionPropertyMapping, NotionRepoConfig};
use crate::domain::notion::{
    NotionApiGateway, NotionConfigStatus, NotionError, NotionLabelOption, NotionPropertyInfo,
    NotionTaskPage, NotionTaskQuery, NotionValidationResult,
};

use super::service_models::{
    build_notion_filter, extract_first_data_source_id, extract_properties_from_json,
    parse_query_response,
};

const NOTION_API_VERSION: &str = "2022-06-28";
const NOTION_BASE_URL: &str = "https://api.notion.com/v1";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_RETRIES: u32 = 2;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NotionApiGatewayImpl;

impl NotionApiGatewayImpl {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl NotionApiGateway for NotionApiGatewayImpl {
    fn query_tasks(
        &self,
        config: &NotionRepoConfig,
        query: &NotionTaskQuery,
    ) -> Result<NotionTaskPage, NotionError> {
        query_tasks(config, query)
    }

    fn fetch_label_options(
        &self,
        config: &NotionRepoConfig,
    ) -> Result<Vec<NotionLabelOption>, NotionError> {
        fetch_label_options(config)
    }

    fn validate(&self, config: &NotionRepoConfig) -> NotionValidationResult {
        validate_config(config)
    }
}

fn build_client(api_token: &str) -> Result<reqwest::blocking::Client, NotionError> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_token}"))
            .map_err(|error| NotionError::RequestFailed(error.to_string()))?,
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
        .map_err(|error| NotionError::RequestFailed(error.to_string()))
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
            .map_err(|error| NotionError::RequestFailed(error.to_string()))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            if retries >= MAX_RETRIES {
                return Err(NotionError::ApiError(
                    "Rate limited after retries".to_string(),
                ));
            }
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
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

fn query_tasks(
    config: &NotionRepoConfig,
    query: &NotionTaskQuery,
) -> Result<NotionTaskPage, NotionError> {
    let client = build_client(&config.api_token)?;
    let url = format!("{NOTION_BASE_URL}/databases/{}/query", config.database_id);
    let body = build_query_body(query, &config.property_mapping);

    let resp = send_with_retry(&client, &url, &body)?;
    let json: serde_json::Value = resp
        .json()
        .map_err(|error| NotionError::ParseError(error.to_string()))?;

    let tasks = parse_query_response(&json, &config.property_mapping)?;
    let (has_more, next_cursor) = parse_page_metadata(&json);

    Ok(NotionTaskPage {
        tasks,
        has_more,
        next_cursor,
    })
}

fn build_query_body(query: &NotionTaskQuery, mapping: &NotionPropertyMapping) -> serde_json::Value {
    let page_size = query.page_size.unwrap_or(20);
    let mut body = serde_json::json!({ "page_size": page_size });

    if let Some(ref cursor) = query.cursor {
        body["start_cursor"] = serde_json::Value::String(cursor.clone());
    }

    if let Some(filter) = build_notion_filter(query, mapping) {
        body["filter"] = filter;
    }

    body
}

fn parse_page_metadata(json: &serde_json::Value) -> (bool, Option<String>) {
    let has_more = json
        .get("has_more")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let next_cursor = json
        .get("next_cursor")
        .and_then(|value| value.as_str())
        .map(String::from);

    (has_more, next_cursor)
}

fn validate_config(config: &NotionRepoConfig) -> NotionValidationResult {
    let client = match build_client(&config.api_token) {
        Ok(client) => client,
        Err(_) => {
            return empty_validation_result(classify_validation_failure(
                ValidationFailure::BuildClient,
            ));
        }
    };

    let url = format!("{NOTION_BASE_URL}/databases/{}", config.database_id);
    let resp = match client.get(&url).send() {
        Ok(resp) => resp,
        Err(_) => {
            return NotionValidationResult {
                status: NotionConfigStatus::NetworkError,
                properties: Vec::new(),
            };
        }
    };

    let status_code = resp.status();
    let status = classify_validation_status(status_code);
    if status != NotionConfigStatus::Configured {
        return empty_validation_result(status);
    }

    let json: serde_json::Value = match resp.json() {
        Ok(json) => json,
        Err(_) => {
            return empty_validation_result(classify_validation_failure(
                ValidationFailure::ParseResponse,
            ));
        }
    };

    let properties = match validation_properties(&json, |data_source_id| {
        fetch_data_source_properties(&client, data_source_id)
    }) {
        Ok(properties) => properties,
        Err(status) => return empty_validation_result(status),
    };

    NotionValidationResult {
        status: NotionConfigStatus::Configured,
        properties,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationFailure {
    BuildClient,
    ParseResponse,
}

fn classify_validation_failure(failure: ValidationFailure) -> NotionConfigStatus {
    match failure {
        ValidationFailure::BuildClient => NotionConfigStatus::InvalidToken,
        ValidationFailure::ParseResponse => NotionConfigStatus::InvalidDatabase,
    }
}

fn classify_validation_status(status_code: reqwest::StatusCode) -> NotionConfigStatus {
    if status_code == reqwest::StatusCode::UNAUTHORIZED {
        return NotionConfigStatus::InvalidToken;
    }

    if status_code == reqwest::StatusCode::TOO_MANY_REQUESTS || status_code.is_server_error() {
        return NotionConfigStatus::NetworkError;
    }

    if status_code == reqwest::StatusCode::NOT_FOUND
        || status_code == reqwest::StatusCode::BAD_REQUEST
        || !status_code.is_success()
    {
        return NotionConfigStatus::InvalidDatabase;
    }

    NotionConfigStatus::Configured
}

fn empty_validation_result(status: NotionConfigStatus) -> NotionValidationResult {
    NotionValidationResult {
        status,
        properties: Vec::new(),
    }
}

fn validation_properties<F>(
    json: &serde_json::Value,
    fetch_data_source_properties: F,
) -> Result<Vec<NotionPropertyInfo>, NotionConfigStatus>
where
    F: FnOnce(&str) -> Result<Vec<NotionPropertyInfo>, NotionError>,
{
    match extract_first_data_source_id(json) {
        Some(data_source_id) => fetch_data_source_properties(&data_source_id)
            .map_err(|_| NotionConfigStatus::NetworkError),
        None => Ok(extract_properties_from_json(json)),
    }
}

fn fetch_label_options(config: &NotionRepoConfig) -> Result<Vec<NotionLabelOption>, NotionError> {
    let client = build_client(&config.api_token)?;
    let props = fetch_database_properties(&client, &config.database_id)?;

    let label_names: HashSet<&str> = config
        .property_mapping
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect();

    let has_people = config
        .property_mapping
        .labels
        .iter()
        .any(|label| label.property_type == "people");

    let workspace_users =
        fetch_workspace_users_for_label_options(has_people, || fetch_workspace_users(&client))?;

    Ok(props
        .into_iter()
        .filter(|property| label_names.contains(property.name.as_str()))
        .map(|property| {
            let mapping_type = config
                .property_mapping
                .labels
                .iter()
                .find(|label| label.name == property.name)
                .map(|label| label.property_type.as_str())
                .unwrap_or("");

            if mapping_type == "people" {
                NotionLabelOption {
                    property_name: property.name,
                    property_type: "people".to_string(),
                    options: workspace_users
                        .iter()
                        .map(|(_, name)| name.clone())
                        .collect(),
                    option_ids: workspace_users.iter().map(|(id, _)| id.clone()).collect(),
                }
            } else {
                NotionLabelOption {
                    property_name: property.name,
                    property_type: property.property_type,
                    options: property.options,
                    option_ids: Vec::new(),
                }
            }
        })
        .collect())
}

fn fetch_workspace_users_for_label_options<F>(
    has_people: bool,
    fetch_workspace_users: F,
) -> Result<Vec<(String, String)>, NotionError>
where
    F: FnOnce() -> Result<Vec<(String, String)>, NotionError>,
{
    if has_people {
        fetch_workspace_users()
    } else {
        Ok(Vec::new())
    }
}

fn fetch_workspace_users(
    client: &reqwest::blocking::Client,
) -> Result<Vec<(String, String)>, NotionError> {
    let mut users = Vec::new();
    let mut next_cursor: Option<String> = None;

    loop {
        let mut url = format!("{NOTION_BASE_URL}/users?page_size=100");
        if let Some(ref cursor) = next_cursor {
            url.push_str(&format!("&start_cursor={cursor}"));
        }

        let resp = client
            .get(&url)
            .send()
            .map_err(|error| NotionError::RequestFailed(error.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(NotionError::ApiError(format!("HTTP {status}: {body}")));
        }

        let json: serde_json::Value = resp
            .json()
            .map_err(|error| NotionError::ParseError(error.to_string()))?;

        if let Some(results) = json.get("results").and_then(|results| results.as_array()) {
            for user in results {
                let user_type = user
                    .get("type")
                    .and_then(|user_type| user_type.as_str())
                    .unwrap_or("");
                if user_type != "person" {
                    continue;
                }
                let id = user
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let name = user
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if !id.is_empty() && !name.is_empty() {
                    users.push((id.to_string(), name.to_string()));
                }
            }
        }

        let has_more = json
            .get("has_more")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !has_more {
            break;
        }
        next_cursor = json
            .get("next_cursor")
            .and_then(|value| value.as_str())
            .map(String::from);
    }

    Ok(users)
}

fn fetch_data_source_properties(
    client: &reqwest::blocking::Client,
    data_source_id: &str,
) -> Result<Vec<NotionPropertyInfo>, NotionError> {
    let url = format!("{NOTION_BASE_URL}/data_sources/{data_source_id}");
    let resp = client
        .get(&url)
        .send()
        .map_err(|error| NotionError::RequestFailed(error.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(NotionError::ApiError(format!("HTTP {status}: {body}")));
    }

    let json: serde_json::Value = resp
        .json()
        .map_err(|error| NotionError::ParseError(error.to_string()))?;

    Ok(extract_properties_from_json(&json))
}

fn fetch_database_properties(
    client: &reqwest::blocking::Client,
    database_id: &str,
) -> Result<Vec<NotionPropertyInfo>, NotionError> {
    let url = format!("{NOTION_BASE_URL}/databases/{database_id}");
    let resp = client
        .get(&url)
        .send()
        .map_err(|error| NotionError::RequestFailed(error.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(NotionError::ApiError(format!("HTTP {status}: {body}")));
    }

    let json: serde_json::Value = resp
        .json()
        .map_err(|error| NotionError::ParseError(error.to_string()))?;

    match extract_first_data_source_id(&json) {
        Some(data_source_id) => fetch_data_source_properties(client, &data_source_id),
        None => Ok(extract_properties_from_json(&json)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use reqwest::StatusCode;

    use super::*;

    fn query(cursor: Option<&str>, page_size: Option<u32>) -> NotionTaskQuery {
        NotionTaskQuery {
            title_filter: String::new(),
            label_filters: HashMap::new(),
            cursor: cursor.map(String::from),
            page_size,
        }
    }

    #[test]
    fn build_query_body_sets_page_size_and_cursor() {
        let body = build_query_body(
            &query(Some("cursor-abc"), Some(50)),
            &NotionPropertyMapping::default(),
        );

        assert_eq!(
            body.get("page_size").and_then(|value| value.as_u64()),
            Some(50)
        );
        assert_eq!(
            body.get("start_cursor").and_then(|value| value.as_str()),
            Some("cursor-abc")
        );
    }

    #[test]
    fn build_query_body_omits_cursor_and_keeps_existing_default_page_size() {
        let body = build_query_body(&query(None, None), &NotionPropertyMapping::default());

        assert_eq!(
            body.get("page_size").and_then(|value| value.as_u64()),
            Some(20)
        );
        assert!(body.get("start_cursor").is_none());
    }

    #[test]
    fn parse_page_metadata_extracts_has_more_and_next_cursor() {
        let json = serde_json::json!({
            "results": [],
            "has_more": true,
            "next_cursor": "cursor-next"
        });

        let (has_more, next_cursor) = parse_page_metadata(&json);

        assert!(has_more);
        assert_eq!(next_cursor.as_deref(), Some("cursor-next"));
    }

    #[test]
    fn parse_page_metadata_defaults_missing_values() {
        let json = serde_json::json!({ "results": [] });

        let (has_more, next_cursor) = parse_page_metadata(&json);

        assert!(!has_more);
        assert!(next_cursor.is_none());
    }

    #[test]
    fn classify_validation_status_matches_behavior_rules() {
        assert_eq!(
            classify_validation_status(StatusCode::UNAUTHORIZED),
            NotionConfigStatus::InvalidToken
        );
        assert_eq!(
            classify_validation_status(StatusCode::NOT_FOUND),
            NotionConfigStatus::InvalidDatabase
        );
        assert_eq!(
            classify_validation_status(StatusCode::BAD_REQUEST),
            NotionConfigStatus::InvalidDatabase
        );
        assert_eq!(
            classify_validation_status(StatusCode::TOO_MANY_REQUESTS),
            NotionConfigStatus::NetworkError
        );
        assert_eq!(
            classify_validation_status(StatusCode::INTERNAL_SERVER_ERROR),
            NotionConfigStatus::NetworkError
        );
        assert_eq!(
            classify_validation_status(StatusCode::OK),
            NotionConfigStatus::Configured
        );
    }

    #[test]
    fn classify_validation_failures_match_fallback_rules() {
        let build_client_failure =
            empty_validation_result(classify_validation_failure(ValidationFailure::BuildClient));
        assert_eq!(
            build_client_failure.status,
            NotionConfigStatus::InvalidToken
        );
        assert!(build_client_failure.properties.is_empty());

        let parse_failure = empty_validation_result(classify_validation_failure(
            ValidationFailure::ParseResponse,
        ));
        assert_eq!(parse_failure.status, NotionConfigStatus::InvalidDatabase);
        assert!(parse_failure.properties.is_empty());
    }

    #[test]
    fn validation_properties_data_source取得失敗はnetwork_errorを返す() {
        let json = serde_json::json!({
            "data_sources": [{ "id": "ds-1" }]
        });

        let result = validation_properties(&json, |_| {
            Err(NotionError::RequestFailed("timeout".to_string()))
        });

        assert_eq!(result.unwrap_err(), NotionConfigStatus::NetworkError);
    }

    #[test]
    fn validation_properties_data_sourceがない場合はdatabase_jsonから抽出する() {
        let json = serde_json::json!({
            "properties": {
                "Name": {
                    "type": "title",
                    "title": {}
                }
            }
        });

        let result =
            validation_properties(&json, |_| panic!("data source fetch should not be called"))
                .unwrap();

        assert_eq!(
            result,
            vec![NotionPropertyInfo {
                name: "Name".to_string(),
                property_type: "title".to_string(),
                options: Vec::new(),
            }]
        );
    }

    #[test]
    fn fetch_workspace_users_for_label_options_people取得失敗を伝播する() {
        let result = fetch_workspace_users_for_label_options(true, || {
            Err(NotionError::ApiError("HTTP 500".to_string()))
        });

        assert_eq!(result.unwrap_err().to_string(), "API エラー: HTTP 500");
    }

    #[test]
    fn fetch_workspace_users_for_label_options_people以外ではusersを取得しない() {
        let result = fetch_workspace_users_for_label_options(false, || {
            panic!("workspace users fetch should not be called")
        })
        .unwrap();

        assert!(result.is_empty());
    }
}
