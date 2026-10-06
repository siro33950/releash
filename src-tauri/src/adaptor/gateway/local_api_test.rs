use super::*;

#[test]
pub fn test_local_api接続先確認_応答の状態を確認済みと別応答と応答なしに分ける() {
    // Given / When / Then
    assert_eq!(
        identity_response(Some(204)),
        ConnectionObservation::IdentityVerified
    );
    for status in [200, 401, 404] {
        assert_eq!(
            identity_response(Some(status)),
            ConnectionObservation::UnexpectedResponse
        );
    }
    assert_eq!(identity_response(None), ConnectionObservation::NoResponse);
}
