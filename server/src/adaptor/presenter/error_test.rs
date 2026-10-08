use super::*;

#[test]
fn test_表示失敗_cause設定でstatusと文言を保ち二重に包まない() {
    // Given
    for source in [
        AppError::new("message"),
        AppError::new("message").with_status(connectrpc::ErrorCode::Unavailable),
    ] {
        let kind = source.connect_code();
        // When
        let result = source.with_cause(Some("source failure".into()));
        // Then
        assert_eq!(result.connect_code(), kind);
        assert_eq!(result.cause(), Some("source failure"));
        assert_eq!(result.to_string(), "message");
        let AppError::Presented { error, .. } = result else {
            panic!("expected presented")
        };
        assert!(matches!(*error, AppError::Internal(_)));
    }
}
pub(crate) mod code_error_tests {
    use super::super::*;

    #[test]
    fn code由来のdisplayが変換チェーンを通じて保持される() {
        // CodeError → CodeUsecaseError → AppError の変換でメッセージ文字列が保持され、
        // serialize 表現が移行前（GitError のプレーン文字列）と等価であることをガードする。
        let usecase_err = CodeUsecaseError::Code(CodeError::Rule("file not staged".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "file not staged");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"file not staged\""
        );
    }

    #[test]
    fn external由来のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = CodeUsecaseError::Code(CodeError::External("git2 boom".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "git2 boom");
        assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
    }

    #[test]
    fn test_review_group操作_staleな対象はcodeと利用者向けmessageを持つerrorとして返す() {
        // Given
        let internal_group_id = "g:old:0";
        let usecase_err = CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget {
            group_id: internal_group_id.to_string(),
        });

        // When
        let app_err = AppError::from(usecase_err);

        // Then
        assert_eq!(app_err.to_string(), STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE);
        assert_eq!(
            serde_json::to_value(&app_err).unwrap(),
            serde_json::json!({
                "code": STALE_REVIEW_GROUP_TARGET_ERROR_CODE,
                "message": STALE_REVIEW_GROUP_TARGET_ERROR_MESSAGE
            })
        );
        assert!(!app_err.to_string().contains(internal_group_id));
        assert!(!app_err.to_string().contains("review group target stale"));
    }
}
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn serializeはプレーン文字列を返す() {
        // 移行前の GitError（プレーン文字列）と等価な観測表現の契約。
        // オブジェクトに包まず、メッセージそのものを JSON 文字列として返す。
        let err = AppError::new("リポジトリパスが設定されていません");
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, "\"リポジトリパスが設定されていません\"");
    }

    #[test]
    fn displayはメッセージそのもの() {
        let err = AppError::new("boom");
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn coded_errorはcodeとmessageを返す() {
        let err = AppError::coded(
            "STALE_REVIEW_GROUP_TARGET",
            "review group target stale: g:old",
            connectrpc::ErrorCode::Aborted,
        );
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "STALE_REVIEW_GROUP_TARGET");
        assert_eq!(json["message"], "review group target stale: g:old");
        assert_eq!(err.to_string(), "review group target stale: g:old");
    }
}
