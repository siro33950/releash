use super::*;
use crate::domain::comment::{Author, ReviewEvent, State, Target};

struct Fixture {
    _directory: tempfile::TempDir,
    data: PathBuf,
    main: String,
    worktree: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        let main = directory.path().join("repo");
        let worktree = directory.path().join("feature");
        let repository = git2::Repository::init(&main).unwrap();
        let tree = repository.index().unwrap().write_tree().unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        let commit = repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &repository.find_tree(tree).unwrap(),
                &[],
            )
            .unwrap();
        let branch = repository
            .branch("feature", &repository.find_commit(commit).unwrap(), false)
            .unwrap();
        repository
            .worktree(
                "feature",
                &worktree,
                Some(git2::WorktreeAddOptions::new().reference(Some(branch.get()))),
            )
            .unwrap();
        let main = main.canonicalize().unwrap().to_str().unwrap().to_string();
        let worktree = worktree
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let mut config =
            crate::adaptor::gateway::app_config::config_models::ReleashConfig::default();
        config.app.last_repo_paths = vec![main.clone()];
        crate::adaptor::gateway::app_config::repository_impl::write_config(
            &data.join("releash.toml"),
            &config,
        )
        .unwrap();
        Self {
            _directory: directory,
            data,
            main,
            worktree,
        }
    }

    fn save(&self, worktree: &str, events: &[ReviewEvent]) {
        super::super::FileReviewEventStore::default()
            .write_events(&self.data, worktree, events)
            .unwrap();
    }
}

fn created(id: &str) -> ReviewEvent {
    ReviewEvent::ThreadCreated {
        event_id: uuid::Uuid::new_v4().to_string(),
        thread_id: id.into(),
        comment_id: uuid::Uuid::new_v4().to_string(),
        actor: Author::human(),
        target: Target {
            file_path: None,
            line_number: None,
            end_line: None,
            side: None,
            end_side: None,
        },
        content: format!("本文-{id}"),
        at: 1.0,
    }
}

#[test]
fn test_thread取得_idの順で別々のworktreeから復元する() {
    // Given
    let fixture = Fixture::new();
    let main_id = uuid::Uuid::new_v4().to_string();
    let worktree_id = uuid::Uuid::new_v4().to_string();
    fixture.save(&fixture.main, &[created(&main_id)]);
    fixture.save(
        &fixture.worktree,
        &[
            created(&worktree_id),
            ReviewEvent::CommentAppended {
                event_id: uuid::Uuid::new_v4().to_string(),
                thread_id: worktree_id.clone(),
                comment_id: uuid::Uuid::new_v4().to_string(),
                actor: Author::human(),
                content: "返信".into(),
                at: 2.0,
            },
            ReviewEvent::ThreadResolved {
                event_id: uuid::Uuid::new_v4().to_string(),
                thread_id: worktree_id.clone(),
                actor: Author::human(),
                outcome: "done".into(),
                summary: "解決".into(),
                at: 3.0,
            },
        ],
    );
    // When
    let threads = FileThreadRepository::new(fixture.data.clone())
        .find_by_ids(&[worktree_id.clone(), main_id.clone(), worktree_id.clone()])
        .unwrap();
    // Then
    assert_eq!(
        threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect::<Vec<_>>(),
        [worktree_id.as_str(), main_id.as_str(), worktree_id.as_str()]
    );
    assert_eq!(threads[0].worktree_name, fixture.worktree);
    assert_eq!(threads[1].worktree_name, fixture.main);
    assert_eq!(threads[0].comments.len(), 2);
    assert_eq!(
        threads[0].comments[0].content,
        format!("本文-{worktree_id}")
    );
    assert_eq!(threads[0].state, State::Resolved);
}

#[test]
fn test_thread取得_存在しないidか削除済みidが一つでもあれば失敗する() {
    // Given
    let fixture = Fixture::new();
    let id = uuid::Uuid::new_v4().to_string();
    let deleted = uuid::Uuid::new_v4().to_string();
    fixture.save(
        &fixture.main,
        &[
            created(&id),
            created(&deleted),
            ReviewEvent::ThreadDeleted {
                event_id: uuid::Uuid::new_v4().to_string(),
                thread_id: deleted.clone(),
                actor: Author::human(),
                at: 2.0,
            },
        ],
    );
    let repository = FileThreadRepository::new(fixture.data);
    for missing in [uuid::Uuid::new_v4().to_string(), deleted] {
        // When
        let result = repository.find_by_ids(&[id.clone(), missing.clone()]);
        // Then
        assert!(matches!(result, Err(Error::NotFound(message)) if message.contains(&missing)));
    }
}

#[test]
fn test_thread取得_登録が無い場合はnotfoundで空の要求は空を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repository = FileThreadRepository::new(directory.path().into());
    // When / Then
    assert!(repository.find_by_ids(&[]).unwrap().is_empty());
    assert!(matches!(
        repository.find_by_ids(&["missing".into()]),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn test_thread取得_壊れた保存データをnotfoundにしない() {
    // Given
    let fixture = Fixture::new();
    fixture.save(&fixture.main, &[]);
    std::fs::write(
        super::super::state_file(&fixture.data, &fixture.main),
        "invalid",
    )
    .unwrap();
    // When
    let result = FileThreadRepository::new(fixture.data).find_by_ids(&["missing".into()]);
    // Then
    assert!(matches!(result, Err(Error::Serialize(_))));
}

#[test]
fn test_thread取得_設定かrepositoryの読み取り失敗をnotfoundにしない() {
    // Given
    let fixture = Fixture::new();
    std::fs::remove_dir_all(&fixture.main).unwrap();
    // When / Then
    assert!(matches!(
        FileThreadRepository::new(fixture.data.clone()).find_by_ids(&["missing".into()]),
        Err(Error::Technical(_) | Error::Io(_))
    ));
    std::fs::write(fixture.data.join("releash.toml"), "invalid = [").unwrap();
    assert!(matches!(
        FileThreadRepository::new(fixture.data).find_by_ids(&["missing".into()]),
        Err(Error::Io(_))
    ));
}
