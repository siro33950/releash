use super::*;

#[test]
fn test_githubのorigin_各接続形式からownerと名前を読む() {
    // Given
    for url in [
        "https://github.com/siro33950/releash.git",
        "https://github.com/siro33950/releash",
        "git@github.com:siro33950/releash.git",
        "ssh://git@github.com/siro33950/releash.git",
    ] {
        // When
        let repository = GitHubRepository::from_origin_url(url);
        // Then
        assert_eq!(
            repository,
            Some(GitHubRepository {
                owner: "siro33950".into(),
                name: "releash".into(),
            }),
            "{url}"
        );
    }
}

#[test]
fn test_githubのorigin_別ホストとリポジトリを指さない値を拒む() {
    // Given
    for url in [
        "https://gitlab.com/user/repo.git",
        "https://github.com.example.com/user/repo.git",
        "https://example.com/github.com/user/repo.git",
        "https://github.com@example.com/user/repo.git",
        "git@example.com:user/repo.git",
        "https://github.com",
        "https://github.com/user",
        "https://github.com/user/",
        "",
        "/local/github.com/user/repo",
    ] {
        // When
        let repository = GitHubRepository::from_origin_url(url);
        // Then
        assert_eq!(repository, None, "{url}");
    }
}

fn source(path: &str, origin_url: Option<&str>) -> RepositoryNameSource {
    RepositoryNameSource {
        path: path.into(),
        origin_url: origin_url.map(str::to_owned),
    }
}

#[test]
fn test_repository名_重複がなければoriginの名前かフォルダ名を登録順に返す() {
    // Given
    let sources = [
        source("/work/local", Some("git@github.com:siro33950/releash.git")),
        source("/work/articles", None),
        source("/work/notes", Some("https://gitlab.com/user/remote.git")),
    ];
    // When
    let names = repository_names(&sources);
    // Then
    assert_eq!(names, ["releash", "articles", "notes"]);
}

#[test]
fn test_repository名_同名のgithubリポジトリだけownerを付ける() {
    // Given
    let sources = [
        source("/work/a", Some("git@github.com:siro33950/releash.git")),
        source("/work/b", Some("https://github.com/example/releash.git")),
        source("/work/c", Some("https://github.com/example/unique.git")),
    ];
    // When
    let names = repository_names(&sources);
    // Then
    assert_eq!(names, ["siro33950/releash", "example/releash", "unique"]);
}

#[test]
fn test_repository名_フォルダ名との重複でもgithub側だけownerを付ける() {
    // Given
    let sources = [
        source("/work/a", Some("https://github.com/example/releash.git")),
        source("/work/releash", None),
        source("/other/releash", Some("https://gitlab.com/other/repo.git")),
    ];
    // When
    let names = repository_names(&sources);
    // Then
    assert_eq!(names, ["example/releash", "releash", "releash"]);
}

#[test]
fn test_repository名_登録がなければ空になる() {
    // Given
    let sources = [];
    // When
    let names = repository_names(&sources);
    // Then
    assert!(names.is_empty());
}
