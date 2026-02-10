use super::error::GitError;
use super::types::CommitInfo;
use git2::{Repository, Sort};

#[tauri::command]
pub fn get_git_log(repo_path: String, limit: Option<usize>) -> Result<Vec<CommitInfo>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let limit = limit.unwrap_or(50);

    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let mut revwalk = repo.revwalk()?;
    revwalk.push(
        head.target()
            .ok_or_else(|| GitError::Custom("HEAD has no target".to_string()))?,
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
        commits.push(CommitInfo {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;

    #[test]
    fn test_get_git_log() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second commit");
        add_and_commit(&repo, "b.txt", "b", "third commit");

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(result.len(), 3);

        let messages: Vec<&str> = result.iter().map(|c| c.message.as_str()).collect();
        assert!(messages.contains(&"initial commit"));
        assert!(messages.contains(&"second commit"));
        assert!(messages.contains(&"third commit"));

        assert_eq!(result[0].author_name, "Test User");
        assert_eq!(result[0].short_hash.len(), 7);
    }

    #[test]
    fn test_get_git_log_with_limit() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second");

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), Some(1)).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_get_git_log_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert!(result.is_empty());
    }
}
