use super::*;

fn valid_content() -> DiscoveryContent {
    DiscoveryContent::new(
        43123,
        "secret".to_string(),
        "instance-1".to_string(),
        42,
        123,
    )
}

#[test]
fn test_discovery内容判定_空または0の値を拒否する() {
    // Given
    let cases = [
        DiscoveryContent::new(0, "secret".to_string(), "instance-1".to_string(), 42, 123),
        DiscoveryContent::new(43123, " ".to_string(), "instance-1".to_string(), 42, 123),
        DiscoveryContent::new(43123, "secret".to_string(), " ".to_string(), 42, 123),
        DiscoveryContent::new(
            43123,
            "secret".to_string(),
            "instance-1".to_string(),
            0,
            123,
        ),
        DiscoveryContent::new(43123, "secret".to_string(), "instance-1".to_string(), 42, 0),
    ];

    // When / Then
    for content in cases {
        assert_eq!(
            DiscoveryAdmissionService::assess_process(&content, ProcessObservation::StartedAt(123),),
            Err(DiscoveryRejection::InvalidOrStale)
        );
    }
}

#[test]
fn test_process観測_参照不能と対象不在と開始時刻を区別する() {
    // Given / When
    let unavailable = ProcessObservation::from_raw(false, None);
    let missing = ProcessObservation::from_raw(true, None);
    let zero = ProcessObservation::from_raw(true, Some(0));
    let found = ProcessObservation::from_raw(true, Some(123));

    // Then
    assert_eq!(unavailable, ProcessObservation::Unavailable);
    assert_eq!(missing, ProcessObservation::ProcessNotFound);
    assert_eq!(zero, ProcessObservation::ProcessNotFound);
    assert_eq!(found, ProcessObservation::StartedAt(123));
}

#[test]
fn test_discovery_process判定_参照不能を専用失敗として拒否する() {
    // Given / When
    let result = DiscoveryAdmissionService::assess_process(
        &valid_content(),
        ProcessObservation::Unavailable,
    );

    // Then
    assert_eq!(
        result,
        Err(DiscoveryRejection::ProcessInformationUnavailable)
    );
}

#[test]
fn test_discovery_process判定_対象不在と開始時刻不一致を陳腐化として拒否する() {
    // Given
    let content = valid_content();

    // When / Then
    assert_eq!(
        DiscoveryAdmissionService::assess_process(&content, ProcessObservation::ProcessNotFound,),
        Err(DiscoveryRejection::InvalidOrStale)
    );
    assert_eq!(
        DiscoveryAdmissionService::assess_process(&content, ProcessObservation::StartedAt(124),),
        Err(DiscoveryRejection::InvalidOrStale)
    );
}

#[test]
fn test_discovery_process判定_開始時刻一致を受理する() {
    // Given / When
    let result = DiscoveryAdmissionService::assess_process(
        &valid_content(),
        ProcessObservation::StartedAt(123),
    );

    // Then
    assert_eq!(result, Ok(()));
}

#[test]
fn test_接続先観測_status204とその他と応答なしを区別する() {
    // Given / When
    let verified = ConnectionObservation::from_response_status(Some(204));
    let mismatch = ConnectionObservation::from_response_status(Some(404));
    let unreachable = ConnectionObservation::from_response_status(None);

    // Then
    assert_eq!(verified, ConnectionObservation::IdentityVerified);
    assert_eq!(mismatch, ConnectionObservation::UnexpectedResponse);
    assert_eq!(unreachable, ConnectionObservation::NoResponse);
}

#[test]
fn test_discovery接続先判定_観測結果を受理または専用失敗へ分類する() {
    // Given / When / Then
    assert_eq!(
        DiscoveryAdmissionService::assess_connection(ConnectionObservation::IdentityVerified,),
        Ok(())
    );
    assert_eq!(
        DiscoveryAdmissionService::assess_connection(ConnectionObservation::UnexpectedResponse,),
        Err(DiscoveryRejection::InstanceMismatch)
    );
    assert_eq!(
        DiscoveryAdmissionService::assess_connection(ConnectionObservation::NoResponse),
        Err(DiscoveryRejection::ConnectionUnreachable)
    );
}
