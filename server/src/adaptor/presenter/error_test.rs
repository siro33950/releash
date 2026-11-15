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

mod repository_error_tests {
    use super::*;
    use crate::domain::git::GitError;
    use crate::usecase::repository_error::UsecaseError;

    #[test]
    fn rule違反のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = UsecaseError::Rule("既定ブランチは削除できません".to_string());
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "既定ブランチは削除できません");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"既定ブランチは削除できません\""
        );
    }

    #[test]
    fn repository_external由来のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = UsecaseError::Repository(GitError::External("git2 boom".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "git2 boom");
        assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
    }

    #[test]
    fn test_commit後失敗_分類と文を保ちcodeを付ける() {
        use crate::adaptor::presenter::connect::ConnectFailure;
        // Given
        for error in [
            UsecaseError::Rule("rule failed".into()),
            UsecaseError::Repository(GitError::External("push failed".into())),
        ] {
            let kind = error.connect_code();
            let message = error.to_string();
            // When
            let converted = AppError::from(UsecaseError::after_commit(error));
            // Then
            assert_eq!(converted.connect_code(), kind);
            assert_eq!(converted.to_string(), message);
            assert_eq!(
                serde_json::to_value(converted).unwrap(),
                serde_json::json!({"code": "GIT_COMMIT_FOLLOW_UP_FAILED", "message": message})
            );
        }
    }
}

#[test]
fn test_多段の結果_成功値をそのまま返す() {
    // Given
    let result = Ok(Ok(Ok(String::from("created"))));
    // When
    let actual = AppError::from_layered::<_, AppError, AppError, AppError>(result);
    // Then
    assert_eq!(actual.unwrap(), "created");
}

#[test]
fn test_多段の結果_各段の失敗のコードと文言を保つ() {
    use crate::domain::git::GitError;
    use crate::domain::git_host::{GitHostError, PrCommentError, PullRequestCreationError};
    // Given
    let business = PullRequestCreationError::EmptyTitle;
    let host = GitHostError::External("denied".into());
    let repository = GitError::Rule("missing branch".into());
    let cases = [
        (
            Ok(Ok(Err(business.clone()))),
            AppError::from_failure(business),
        ),
        (Ok(Err(host.clone())), AppError::from_failure(host)),
        (Err(repository.clone()), AppError::from_failure(repository)),
    ];
    for (result, expected) in cases {
        // When
        let actual = AppError::from_layered::<(), _, _, _>(result).unwrap_err();
        // Then
        assert_eq!(actual.connect_code(), expected.connect_code());
        assert_eq!(actual.to_string(), expected.to_string());
    }
    // Given
    let result = Ok(Ok(Err(PrCommentError::Empty)));
    // When
    let actual = AppError::from_layered::<(), _, GitHostError, GitError>(result).unwrap_err();
    // Then
    assert_eq!(
        actual.connect_code(),
        connectrpc::ErrorCode::InvalidArgument
    );
    assert_eq!(actual.to_string(), PrCommentError::Empty.to_string());
}
