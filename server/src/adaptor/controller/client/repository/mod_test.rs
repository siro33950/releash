use super::*;
use crate::domain::git::GitError;

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
