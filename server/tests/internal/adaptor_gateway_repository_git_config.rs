use releashd::test_support::integration::repository::get_releash_base;
use releashd::test_support::integration::repository::resolve_base_commit_oid;
use releashd::test_support::integration::repository::resolve_effective_base_branch_with_git;
use releashd::test_support::integration::repository::set_branch_base_override;
use releashd::test_support::integration::repository::set_releash_base;
use releashd::test_support::integration::repository::GitService;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_git設定_各操作の停止を既定値に変えず後続へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| settings().get_branch_base(path, "main"));
    assert_stops_at_each_checkpoint(|| get_releash_base(path));
    assert_stops_at_each_checkpoint(|| set_releash_base(path, Some("main")));
    assert_stops_at_each_checkpoint(|| set_branch_base_override(path, "stale", Some("main")));
    assert_stops_at_each_checkpoint(|| {
        let repo = git2::Repository::open(path).unwrap();
        let mut config = repo.config().unwrap();
        for branch in ["stale", "stale.two"] {
            config
                .set_str(&format!("branch.{branch}.releash-base"), "main")
                .unwrap();
        }
        settings().prune_stale_branch_bases(path, &[])
    });
}

#[test]
pub fn test_base解決_各停止点で欠損へ変換せず次の候補へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    let oid = create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    repo.config()
        .unwrap()
        .set_str("releash.base", "base")
        .unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| git().resolve_current_base_branch(path));
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "missing"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    repo.reference("refs/remotes/origin/base", oid, true, "test")
        .unwrap();
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "base"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    repo.branch("base", &repo.find_commit(oid).unwrap(), false)
        .unwrap();
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "base"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    let deleted = dir.path().join("deleted");
    assert_stops_at_each_checkpoint(|| {
        git().resolve_current_base_branch(deleted.to_str().unwrap())
    });
}

#[test]
pub fn test_base読取_設定破損を未設定や既定branchに変換しない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    std::fs::write(repo.path().join("config"), "[broken\n").unwrap();
    let path = dir.path().to_str().unwrap();
    // When
    let branch_base = settings().get_branch_base(path, &branch);
    let releash_base = get_releash_base(path);
    let current_base = git().resolve_current_base_branch(path);
    let effective_base = resolve_effective_base_branch(path);
    // Then
    assert!(branch_base.is_err());
    assert!(releash_base.is_err());
    assert!(current_base.is_err());
    assert!(effective_base.is_err());
}

#[test]
pub fn test_base読取_コミットがないbranchは未設定を返す() {
    // Given
    let (dir, _repo) = create_test_repo();
    // When
    let result = settings().get_branch_base(dir.path().to_str().unwrap(), "unborn");
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_base読取_正常な設定から現在branchと既定baseを読む() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    // When
    let branch_base = settings().get_branch_base(path, &branch);
    let releash_base = get_releash_base(path);
    let effective_base = resolve_effective_base_branch(path);
    // Then
    assert_eq!(branch_base.unwrap(), Some(branch));
    assert_eq!(releash_base.unwrap(), None);
    assert!(effective_base.is_ok());
}

#[test]
pub fn test_git任意読取_resolve_effective_base_branch_リポジトリでないディレクトリは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().to_path_buf();
    // When
    let result = resolve_effective_base_branch(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_git任意読取_resolve_effective_base_branch_存在しないパスは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().join("missing");
    // When
    let result = resolve_effective_base_branch(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}
pub(crate) mod git_config_gateway_tests {
    use super::*;

    use releashd::test_support::integration::repository::get_releash_base;
    use releashd::test_support::integration::repository::set_branch_base_override;
    use releashd::test_support::integration::repository::set_releash_base;

    #[test]
    pub fn test_ベース解決_per_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let result = settings()
            .get_branch_base(repo.workdir().unwrap().to_str().unwrap(), "feat")
            .unwrap();
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    pub fn test_ベース解決_releash_baseフォールバック() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_releash_base(repo_path, Some("develop")).unwrap();
        let result = settings()
            .get_branch_base(repo.workdir().unwrap().to_str().unwrap(), "feat")
            .unwrap();
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    pub fn test_ベース解決_既定ブランチフォールバック() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let result = settings()
            .get_branch_base(repo.workdir().unwrap().to_str().unwrap(), "feat")
            .unwrap();
        assert!(result.is_some());
    }

    #[test]
    pub fn test_branch_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        let base = settings().get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let base = settings().get_branch_base(repo_path, "feat").unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_branch_base_override(repo_path, "feat", None).unwrap();
        let base = settings().get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());
        assert_ne!(base, Some("develop".to_string()));
    }

    #[test]
    pub fn test_releash_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap();

        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);

        set_releash_base(repo_path, Some("develop")).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_releash_base(repo_path, None).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);
    }

    fn checkout_feature_branch(repo: &git2::Repository) {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }

    #[test]
    pub fn test_現在ブランチbase解決_override優先() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        checkout_feature_branch(&repo);
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();

        let result = git().resolve_current_base_branch(repo_path).unwrap();
        assert_eq!(result, Some(default_branch));
    }

    #[test]
    pub fn test_現在ブランチbase解決_detached_none() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);
        repo.set_head_detached(oid).unwrap();

        let result = git()
            .resolve_current_base_branch(dir.path().to_str().unwrap())
            .unwrap();
        assert_eq!(result, None);
    }

    #[test]
    pub fn test_実効base_ref実在ならsome_ref不在ならnone() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        checkout_feature_branch(&repo);
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();
        assert_eq!(
            resolve_effective_base_branch(repo_path).unwrap(),
            Some(default_branch)
        );

        set_branch_base_override(repo_path, "feature", Some("no-such-branch")).unwrap();
        assert_eq!(resolve_effective_base_branch(repo_path).unwrap(), None);
    }

    #[test]
    pub fn test_実効base_merge_base不成立ならnone() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();
        let signature = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let orphan = repo
            .commit(None, &signature, &signature, "orphan root", &tree, &[])
            .unwrap();
        repo.reference("refs/heads/feature", orphan, true, "orphan")
            .unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();

        assert_eq!(resolve_effective_base_branch(repo_path).unwrap(), None);
    }

    #[test]
    pub fn test_gc_現存しないブランチのbaseを掃除() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "alive", Some("main")).unwrap();
        set_branch_base_override(repo_path, "stale", Some("main")).unwrap();

        // "alive" のみ現存ブランチとして渡すと "stale" の base が掃除される
        settings()
            .prune_stale_branch_bases(repo_path, &["alive".to_string()])
            .unwrap();

        let config = repo.config().unwrap();
        assert!(config.get_string("branch.alive.releash-base").is_ok());
        assert!(config.get_string("branch.stale.releash-base").is_err());
    }
}

#[test]
fn test_git設定_親ディレクトリごと削除されたファイルでもbaseを解決する() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let root = repo.workdir().unwrap();
    let base = repo.head().unwrap().shorthand().unwrap().to_owned();
    set_releash_base(root.to_str().unwrap(), Some(&base)).unwrap();
    let deleted = root.join("deleted/parent/file.png");
    // When / Then
    assert_eq!(
        git()
            .resolve_current_base_branch(deleted.to_str().unwrap())
            .unwrap(),
        Some(base.clone())
    );
    assert_eq!(
        resolve_base_commit_oid(deleted.to_str().unwrap(), &base).unwrap(),
        Some(repo.head().unwrap().target().unwrap().to_string())
    );
}

fn config_repository() -> releashd::test_support::integration::settings::AppConfig {
    releashd::test_support::integration::settings::AppConfig::new(
        Default::default(),
        std::path::PathBuf::new(),
    )
}

fn git() -> releashd::test_support::integration::repository::GitGateway {
    releashd::test_support::integration::repository::GitGateway::new(std::sync::Arc::new(
        config_repository(),
    ))
}

fn settings() -> releashd::test_support::integration::settings::RepositoryConfigUsecase {
    use releashd::test_support::integration::{
        repository::GitGateway, settings::RepositoryConfigUsecase,
    };
    let config = std::sync::Arc::new(config_repository());
    RepositoryConfigUsecase::new(
        config.clone(),
        std::sync::Arc::new(GitGateway::new(config)),
        Default::default(),
    )
}

#[test]
fn test_repository設定_全ての指定とnotionを復元し既定branchを書き戻さない() {
    use releashd::test_support::integration::repository::RepositoryConfigRepository;
    use releashd::test_support::integration::settings::{load_or_create_config, AppConfig};

    // Given
    let (directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = directory.path().to_str().unwrap();
    let default = repo.head().unwrap().shorthand().unwrap().to_owned();
    let config_path = directory.path().join("releash.toml");
    let contents = format!(
        r#"
[notion."{path}"]
api_token = "notion-token"
database_id = "database-id"
[notion."{path}".property_mapping]
title = "Task"
branch_name = "Branch"
branch_prefix = "issue/"
labels = [{{name = "Status", property_type = "status"}}]
"#
    );
    std::fs::write(&config_path, &contents).unwrap();
    let settings = AppConfig::new(
        load_or_create_config(&config_path).unwrap(),
        config_path.clone(),
    );
    let mut git_config = repo.config().unwrap();
    git_config
        .set_str("branch.Feature.v1/topic.releash-base", "develop")
        .unwrap();
    git_config
        .set_str("branch.stale.releash-base", "old")
        .unwrap();
    git_config
        .set_str("branch.ignored.releash-base-extra", "unrelated")
        .unwrap();

    // When
    let config = settings.load(path).unwrap();
    settings.save(config.clone()).unwrap();

    // Then
    assert_eq!(config.repo_path(), path);
    assert_eq!(config.base_branch(), None);
    assert_eq!(
        config.resolve_base_branch("Feature.v1/topic"),
        Some("develop")
    );
    assert_eq!(config.resolve_base_branch("other"), Some(default.as_str()));
    let mut stale = config.stale_branch_overrides(&["Feature.v1/topic".into()]);
    stale.sort();
    assert_eq!(stale, ["stale"]);
    let notion = config.notion().unwrap();
    assert_eq!(notion.api_token, "notion-token");
    assert_eq!(notion.database_id, "database-id");
    assert_eq!(notion.property_mapping.title, "Task");
    assert_eq!(notion.property_mapping.branch_name, "Branch");
    assert_eq!(notion.property_mapping.branch_prefix, "issue/");
    assert_eq!(notion.property_mapping.labels[0].name, "Status");
    assert_eq!(notion.property_mapping.labels[0].property_type, "status");
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), contents);
    assert_eq!(
        git_config.get_string("releash.base").unwrap_err().code(),
        git2::ErrorCode::NotFound
    );
}

#[test]
fn test_worktree削除検証_拒否とforceのどちらも実体を変更しない() {
    use releashd::test_support::integration::repository::create_worktree;

    // Given
    let (directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = directory.path().to_str().unwrap();
    let worktree_path = directory.path().join("linked");
    let worktree_path = worktree_path.to_str().unwrap();
    create_worktree(path, worktree_path, "feature", true, None).unwrap();
    let git = git();

    // When / Then
    assert!(git.validate_removal(path, path, true).is_err());
    assert_eq!(
        git.validate_removal(path, worktree_path, false).unwrap(),
        std::fs::canonicalize(worktree_path)
            .unwrap()
            .to_str()
            .unwrap()
    );
    std::fs::write(std::path::Path::new(worktree_path).join("dirty"), "keep").unwrap();
    assert!(git.validate_removal(path, worktree_path, false).is_err());
    let worktree = repo.find_worktree("linked").unwrap();
    worktree.lock(Some("keep locked")).unwrap();
    assert!(git.validate_removal(path, worktree_path, false).is_err());
    assert!(git.validate_removal(path, worktree_path, true).is_ok());
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(worktree_path).join("dirty")).unwrap(),
        "keep"
    );
    worktree.validate().unwrap();
    assert!(matches!(
        worktree.is_locked().unwrap(),
        git2::WorktreeLockStatus::Locked(_)
    ));
}

#[test]
fn test_現在ブランチbase解決_linked_worktree固有の設定を使う() {
    // Given
    let (_directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let linked_directory = tempfile::tempdir().unwrap();
    let path = linked_directory.path().join("linked");
    repo.worktree("linked", &path, None).unwrap();
    let mut config = repo.config().unwrap();
    config.set_bool("extensions.worktreeConfig", true).unwrap();
    config.set_str("releash.base", "main").unwrap();
    let linked = git2::Repository::open(&path).unwrap();
    std::fs::write(
        linked.path().join("config.worktree"),
        "[releash]\nbase = develop\n",
    )
    .unwrap();
    let linked = git2::Repository::open(&path).unwrap();
    assert_eq!(
        linked.config().unwrap().get_string("releash.base").unwrap(),
        "develop"
    );

    // When
    let base = git()
        .resolve_current_base_branch(path.to_str().unwrap())
        .unwrap();

    // Then
    assert_eq!(base.as_deref(), Some("develop"));
}

#[test]
fn test_現在ブランチbase解決_linked_worktreeがbare由来でも保存済みの設定を使う() {
    // Given
    let (_source_directory, source) = create_test_repo();
    create_initial_commit(&source);
    let directory = tempfile::tempdir().unwrap();
    let bare = git2::build::RepoBuilder::new()
        .bare(true)
        .clone(
            source.workdir().unwrap().to_str().unwrap(),
            &directory.path().join("project.git"),
        )
        .unwrap();
    bare.config()
        .unwrap()
        .set_str("releash.base", "configured")
        .unwrap();
    let path = directory.path().join("linked");
    bare.worktree("linked", &path, None).unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let linked = git2::Repository::open(&path).unwrap();
    assert_eq!(
        linked.config().unwrap().get_string("releash.base").unwrap(),
        "configured"
    );

    // When
    let base = git()
        .resolve_current_base_branch(path.to_str().unwrap())
        .unwrap();

    // Then
    assert_eq!(base.as_deref(), Some("configured"));
}

#[test]
fn test_現在ブランチbase解決_bare_repositoryでも設定を読む() {
    // Given
    let (_directory, repo) = create_test_repo();
    let oid = create_initial_commit(&repo);
    let bare_directory = tempfile::tempdir().unwrap();
    let bare = git2::build::RepoBuilder::new()
        .bare(true)
        .clone(
            repo.workdir().unwrap().to_str().unwrap(),
            &bare_directory.path().join("clone"),
        )
        .unwrap();
    bare.config()
        .unwrap()
        .set_str("releash.base", "configured")
        .unwrap();
    assert_eq!(bare.head().unwrap().target(), Some(oid));

    // When / Then
    assert_eq!(
        git()
            .resolve_current_base_branch(bare.path().to_str().unwrap())
            .unwrap(),
        Some("configured".into())
    );
}

#[test]
fn test_現在ブランチbase解決_注入されたentityと読み込みの失敗をそのまま使う() {
    use releashd::test_support::integration::platform::{TechnicalFailure, TechnicalFailureNature};
    use releashd::test_support::integration::repository::{
        GitError, GitGateway, RepositoryConfig, RepositoryConfigRepository,
    };
    use releashd::test_support::integration::settings::AppConfigError;

    struct Config(Result<RepositoryConfig, AppConfigError>);
    impl RepositoryConfigRepository for Config {
        fn load(&self, _: &str) -> Result<RepositoryConfig, AppConfigError> {
            self.0.clone()
        }
        fn save(&self, _: RepositoryConfig) -> Result<(), AppConfigError> {
            panic!("read must not save")
        }
    }

    // Given
    let (directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = directory.path().to_str().unwrap();
    repo.config()
        .unwrap()
        .set_str("releash.base", "raw-value")
        .unwrap();
    let failure = TechnicalFailure {
        nature: TechnicalFailureNature::Cancelled,
        message: "cancelled reading settings".into(),
    };
    let config = RepositoryConfig::new(
        path.into(),
        Some("injected".into()),
        Default::default(),
        None,
        None,
    );

    // When / Then
    for (loaded, expected) in [
        (Ok(config), Ok(Some("injected".into()))),
        (
            Err(AppConfigError::Technical(failure.clone())),
            Err(GitError::Technical(failure)),
        ),
        (
            Err(AppConfigError::InvalidInput("invalid settings".into())),
            Err(GitError::rule("invalid settings")),
        ),
    ] {
        let gateway = GitGateway::new(std::sync::Arc::new(Config(loaded)));
        assert_eq!(gateway.resolve_current_base_branch(path), expected);
    }
}

fn resolve_effective_base_branch(
    path: &str,
) -> Result<Option<String>, releashd::test_support::integration::repository::GitError> {
    resolve_effective_base_branch_with_git(path, &git())
}
