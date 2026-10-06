pub(crate) mod tests {

    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use crate::adaptor_controller_api_mod::test_support::get_json;
    use crate::adaptor_controller_api_mod::test_support::test_router;

    #[tokio::test]
    pub async fn test_診断api_queryを検証して共通reportを返す() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        std::fs::write(workflows.path().join("custom.yml"), "name: [").unwrap();
        let (router, _, _) = test_router(data.path(), "secret");

        // When
        let applied = get_json(&router, "/v1/workflow/diagnostics").await;

        // Then
        assert_eq!(applied.0, StatusCode::OK);
        assert!(applied.1["items"].is_array());
        for field in ["workflow_summaries", "facet_summaries", "facet_usage"] {
            assert!(applied.1[field].is_object());
        }

        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(workflows.path().as_os_str().as_encoded_bytes())
                .collect();
        let specified = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;
        assert_eq!(specified.0, StatusCode::OK);
        assert!(specified.1["workflow_summaries"]["custom"].is_object());

        for uri in [
            "/v1/workflow/diagnostics?dir=",
            "/v1/workflow/diagnostics?dir=relative/path",
            "/v1/workflow/diagnostics?unknown=1",
        ] {
            let response = get_json(&router, uri).await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST, "uri: {uri}");
        }

        let unauthorized = router
            .oneshot(
                Request::builder()
                    .uri("/v1/workflow/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    pub async fn test_診断api_存在しない指定directoryをnot_foundにする() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let missing = data.path().join("missing");
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(missing.as_os_str().as_encoded_bytes()).collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;

        // Then
        assert_eq!(response.0, StatusCode::NOT_FOUND);
        assert_eq!(response.1["code"], "not_found");
        assert_eq!(
            response.1["message"],
            format!("directory does not exist: {}", missing.display())
        );
        assert!(response.1.get("items").is_none());
    }

    #[tokio::test]
    pub async fn test_診断api_通常fileの指定をinternal_errorにする() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let file = data.path().join("workflow.yml");
        std::fs::write(&file, "name: workflow").unwrap();
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(file.as_os_str().as_encoded_bytes()).collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;

        // Then
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["code"], "workflow_error");
        assert!(response.1.get("items").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    pub async fn test_診断api_列挙不能な指定directoryをinternal_errorにする() {
        use std::os::unix::fs::PermissionsExt;

        // Given
        let data = tempfile::tempdir().unwrap();
        let unreadable = data.path().join("unreadable");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&unreadable).is_ok() {
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!(
                "skipping unreadable directory test because this process can read mode 0o000"
            );
            return;
        }
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(unreadable.as_os_str().as_encoded_bytes())
                .collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();

        // Then
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["code"], "workflow_error");
        assert!(response.1.get("items").is_none());
    }
}
