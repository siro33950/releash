use super::*;
fn discovery(port: u16, token: &str) -> LocalApiDiscovery {
    LocalApiDiscovery {
        port,
        token: token.to_string(),
        instance_id: "test-instance".to_string(),
        pid: 42,
        process_started_at: 123,
    }
}

#[test]
fn test_local_api認証_discoveryのbearer_tokenをrequestへ設定する() {
    // Given
    let client = LocalApiHttpClient::new(discovery(43123, "secret")).unwrap();

    // When
    let request = client
        .authenticated(client.client.get(client.base_url.clone()))
        .build()
        .unwrap();

    // Then
    assert_eq!(
        request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .unwrap(),
        "Bearer secret"
    );
}
