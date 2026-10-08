use super::*;

#[test]
fn is_github_accepts_github_urls() {
    assert!(is_github("https://github.com/user/repo.git"));
    assert!(is_github("git@github.com:user/repo.git"));
}

#[test]
fn is_github_rejects_other_hosts() {
    assert!(!is_github("https://gitlab.com/user/repo.git"));
}
