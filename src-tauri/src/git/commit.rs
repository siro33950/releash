use super::error::GitError;
use git2::{ErrorCode, Repository};
use std::process::Command;

#[tauri::command]
pub fn git_commit(repo_path: String, message: String) -> Result<String, GitError> {
    let repo = Repository::open(&repo_path)?;
    let sig = repo.signature()?;

    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let parents = match repo.head() {
        Ok(head_ref) => {
            let commit = head_ref.peel_to_commit()?;
            vec![commit]
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch => vec![],
        Err(e) => return Err(e.into()),
    };

    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parent_refs)?;

    Ok(oid.to_string())
}

#[tauri::command]
pub fn git_push(repo_path: String) -> Result<String, GitError> {
    Repository::open(&repo_path)?;

    let output = Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| GitError::Custom(format!("Failed to execute git push: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(format!("{stdout}{stderr}").trim().to_string())
    } else {
        Err(GitError::Custom(stderr.trim().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::log::get_git_log;
    use crate::git::stage::git_stage;
    use crate::git::test_helpers::*;
    use std::fs;

    #[test]
    fn test_commit_normal() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let hash = git_commit(
            dir.path().to_str().unwrap().to_string(),
            "test commit".to_string(),
        )
        .unwrap();
        assert_eq!(hash.len(), 40);

        let log = get_git_log(dir.path().to_str().unwrap().to_string(), Some(1)).unwrap();
        assert_eq!(log[0].message, "test commit");
    }

    #[test]
    fn test_commit_initial() {
        let (dir, _repo) = create_test_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let hash = git_commit(
            dir.path().to_str().unwrap().to_string(),
            "first commit".to_string(),
        )
        .unwrap();
        assert_eq!(hash.len(), 40);

        let log = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].message, "first commit");
    }
}
