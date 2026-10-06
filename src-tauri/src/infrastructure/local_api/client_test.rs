use super::*;
use crate::infrastructure::local_api::test_helpers::*;

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
