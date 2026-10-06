use super::*;
use reqwest::StatusCode;
use std::{collections::HashMap, time::Duration};

fn query(cursor: Option<&str>, page_size: Option<u32>) -> NotionTaskQuery {
    NotionTaskQuery {
        title_filter: String::new(),
        label_filters: HashMap::new(),
        cursor: cursor.map(String::from),
        page_size,
    }
}

#[test]
fn test_retry_afterは秒数とhttp日時を扱い不正な値は指定なしになる() {
    let now = std::time::UNIX_EPOCH + Duration::from_secs(1445412480);
    assert_eq!(retry_after("12", now), Some(Duration::from_secs(12)));
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:28:03 GMT", now),
        Some(Duration::from_secs(3))
    );
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:27:59 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(retry_after("invalid", now), None);
    assert_eq!(retry_after("-1", now), None);
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

#[tokio::test]
async fn validation_properties_data_source取得失敗を呼出元へ返す() {
    let json = serde_json::json!({
        "data_sources": [{ "id": "ds-1" }]
    });

    let result = validation_properties(&json, |_| async {
        Err(NotionError::RequestFailed("timeout".to_string()))
    })
    .await;

    assert_eq!(
        result.unwrap_err(),
        NotionError::RequestFailed("timeout".to_string())
    );
}

#[tokio::test]
async fn validation_properties_data_sourceがない場合はdatabase_jsonから抽出する() {
    let json = serde_json::json!({
        "properties": {
            "Name": {
                "type": "title",
                "title": {}
            }
        }
    });

    let result = validation_properties(&json, |_| async {
        panic!("data source fetch should not be called")
    })
    .await
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

#[tokio::test]
async fn fetch_workspace_users_for_label_options_people取得失敗を伝播する() {
    let result = fetch_workspace_users_for_label_options(true, || async {
        Err(NotionError::ApiError("HTTP 500".to_string()))
    })
    .await;

    assert_eq!(result.unwrap_err().to_string(), "API エラー: HTTP 500");
}

#[tokio::test]
async fn fetch_workspace_users_for_label_options_people以外ではusersを取得しない() {
    let result = fetch_workspace_users_for_label_options(false, || async {
        panic!("workspace users fetch should not be called")
    })
    .await
    .unwrap();

    assert!(result.is_empty());
}
