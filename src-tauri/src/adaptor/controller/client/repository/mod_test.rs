use super::*;
use crate::domain::repository::RepositoryError;

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
    let usecase_err = UsecaseError::Repository(RepositoryError::External("git2 boom".to_string()));
    let app_err = AppError::from(usecase_err);
    assert_eq!(app_err.to_string(), "git2 boom");
    assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
}
