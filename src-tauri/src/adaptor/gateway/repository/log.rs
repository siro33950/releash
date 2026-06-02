//! commit・log 責務の gateway 実装。git2 による履歴読み取りを封じ込める。

use crate::domain::repository::{Commit, LogRepository, RepositoryError};
use crate::infrastructure::git::client;
use git2::Sort;

pub(crate) fn get_git_log(
    repo_path: &str,
    limit: Option<usize>,
) -> Result<Vec<Commit>, RepositoryError> {
    let repo = client::open(repo_path)?;
    let limit = limit.unwrap_or(50);

    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut revwalk = repo.revwalk()?;
    revwalk.push(
        head.target()
            .ok_or_else(|| RepositoryError::rule("HEAD has no target"))?,
    )?;
    revwalk.set_sorting(Sort::TIME)?;

    let mut commits = Vec::new();
    for oid in revwalk {
        if commits.len() >= limit {
            break;
        }
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let hash = oid.to_string();
        let short_hash = hash[..7.min(hash.len())].to_string();
        commits.push(Commit {
            hash,
            short_hash,
            message: commit.message().unwrap_or("").to_string(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
        });
    }

    Ok(commits)
}

/// `LogRepository` の git2 実装。
pub struct LogGateway;

impl LogRepository for LogGateway {
    fn log(&self, repo_path: &str, limit: Option<usize>) -> Result<Vec<Commit>, RepositoryError> {
        get_git_log(repo_path, limit)
    }
}

#[cfg(test)]
mod log_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;

    #[test]
    fn test_コミット履歴取得() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second commit");
        add_and_commit(&repo, "b.txt", "b", "third commit");

        let result = get_git_log(dir.path().to_str().unwrap(), None).unwrap();
        assert_eq!(result.len(), 3);

        let messages: Vec<&str> = result.iter().map(|c| c.message.as_str()).collect();
        assert!(messages.contains(&"initial commit"));
        assert!(messages.contains(&"second commit"));
        assert!(messages.contains(&"third commit"));

        assert_eq!(result[0].author_name, "Test User");
        assert_eq!(result[0].short_hash.len(), 7);
    }

    #[test]
    fn test_コミット履歴取得_limit指定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second");

        let result = get_git_log(dir.path().to_str().unwrap(), Some(1)).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_コミット履歴取得_空リポジトリ() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_log(dir.path().to_str().unwrap(), None).unwrap();
        assert!(result.is_empty());
    }
}
