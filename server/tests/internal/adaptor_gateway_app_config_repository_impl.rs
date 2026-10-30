use releashd::test_support::integration::repository::AppConfigRepository;
use releashd::test_support::integration::repository::RepositoryConfigRepository;
use releashd::test_support::integration::settings::config_to_domain;
use releashd::test_support::integration::settings::load_or_create_config;
use releashd::test_support::integration::settings::AppConfig;
use releashd::test_support::integration::settings::AppConfigError;
use releashd::test_support::integration::settings::ReleashConfig;
use std::fs;

use releashd::test_support::integration::repository::ConfigSecretRepository;
use tempfile::TempDir;

#[test]
pub fn test_設定の秘匿対象_旧serverとnotionの単独値を出力とartifactで秘匿する() {
    // Given
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("releash.toml");
    let repo_dir = dir.path().join("repo");
    git2::Repository::init(&repo_dir).unwrap();
    let repo_path = repo_dir.to_str().unwrap();
    let legacy = r#"
[server]
bind = "0.0.0.0"
port = 9700
token = "0123456789abcdef0123456789abcdef"
[server.tls]
enabled = true
cert = "cert.pem"
key = "key.pem"
[app]
close_to_tray = false
[notion."/repo"]
api_token = "notion-value-1234"
database_id = "database-id"
"#;
    let legacy = legacy.replace("/repo", repo_path);
    fs::write(&path, &legacy).unwrap();
    let config = AppConfig::new(load_or_create_config(&path).unwrap(), path.clone());

    // When
    let secrets = config.configured_secret_values().unwrap();

    // Then
    for value in ["0123456789abcdef0123456789abcdef", "notion-value-1234"] {
        assert!(secrets.contains(&value.to_string()));
        assert_eq!(
            releashd::test_support::integration::workflow::mask_sensitive_text(value, &[]),
            value
        );
        assert_eq!(
            releashd::test_support::integration::workflow::mask_sensitive_text(value, &secrets),
            "[REDACTED]"
        );
        assert_eq!(
            releashd::test_support::integration::workflow::mask_sensitive_artifact(
                "result",
                serde_json::json!({"output": [value, {"nested": value}], "count": 1}),
                &secrets,
            ),
            serde_json::json!({"output": ["[REDACTED]", {"nested": "[REDACTED]"}], "count": 1}),
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
    let before = AppConfigRepository::load(&config).unwrap();
    let notion = RepositoryConfigRepository::load(&config, repo_path).unwrap();
    assert_eq!(notion.notion().unwrap().database_id, "database-id");

    // When
    AppConfigRepository::save(&config, before.clone()).unwrap();

    // Then
    let saved = fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("[server"));
    assert!(!saved.contains("0123456789abcdef0123456789abcdef"));
    assert!(!saved.contains("cert.pem"));
    assert_eq!(AppConfigRepository::load(&config).unwrap(), before);
    assert_eq!(
        RepositoryConfigRepository::load(&config, repo_path).unwrap(),
        notion
    );
    assert_eq!(
        config.configured_secret_values().unwrap(),
        ["notion-value-1234"]
    );
    assert_eq!(
        config_to_domain(&load_or_create_config(&path).unwrap()),
        before
    );
}

#[test]
pub fn test_設定の秘匿対象_旧server_tokenの長さと欠損を扱う() {
    for (legacy, expected) in [
        ("", None),
        ("[server]\nport = 9700", None),
        ("[server]\ntoken = ''", None),
        ("[server]\ntoken = '1234567'", None),
        ("[server]\ntoken = '12345678'", Some("12345678")),
        ("[server]\ntoken = 12345678", None),
    ] {
        // Given
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("releash.toml");
        fs::write(&path, legacy).unwrap();
        let config = AppConfig::new(load_or_create_config(&path).unwrap(), path.clone());
        // When
        let secrets = config.configured_secret_values().unwrap();
        // Then
        assert_eq!(secrets, expected.into_iter().collect::<Vec<_>>());
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
    }
}

#[test]
pub fn test_設定の秘匿対象_ファイル不在でもメモリ上のnotionを収集する() {
    // Given
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("releash.toml");
    let config =
        toml::from_str("[notion.'/repo']\napi_token = 'notion-value-1234'\ndatabase_id = 'db'")
            .unwrap();
    let config = AppConfig::new(config, path.clone());
    // When
    let secrets = config.configured_secret_values().unwrap();
    // Then
    assert_eq!(secrets, ["notion-value-1234"]);
    assert!(!path.exists());
}

#[test]
pub fn test_設定の秘匿対象_設定ファイル取得失敗でも収集済みnotionを保持する() {
    for parse_failure in [false, true] {
        // Given
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("releash.toml");
        if parse_failure {
            fs::write(&path, "[server]\ntoken = 'legacy-sensitive-value' invalid").unwrap();
        } else {
            fs::create_dir(&path).unwrap();
        }
        let config =
            toml::from_str("[notion.'/repo']\napi_token = 'notion-value-1234'\ndatabase_id = 'db'")
                .unwrap();
        let config = AppConfig::new(config, path);

        // When
        let secrets = config.configured_secret_values().unwrap();

        // Then
        assert_eq!(secrets, ["notion-value-1234"]);
    }
}

#[test]
pub fn test_設定の秘匿対象_読み取り失敗を通知する() {
    // Given
    let dir = TempDir::new().unwrap();
    let config = AppConfig::new(ReleashConfig::default(), dir.path().to_path_buf());
    // When
    let error = config.read_legacy_server_token().unwrap_err();
    // Then
    assert!(matches!(error, AppConfigError::Technical(_)));
    assert!(error.to_string().contains("読み込み失敗"));
}

#[test]
pub fn test_設定の秘匿対象_パース失敗にtokenを含めず通知する() {
    // Given
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("releash.toml");
    fs::write(&path, "[server]\ntoken = 'legacy-sensitive-value' invalid").unwrap();
    let config = AppConfig::new(ReleashConfig::default(), path);
    // When
    let error = config.read_legacy_server_token().unwrap_err();
    // Then
    assert!(matches!(error, AppConfigError::Technical(_)));
    assert!(error.to_string().contains("パース失敗"));
    assert!(!error.to_string().contains("legacy-sensitive-value"));
}

pub(crate) mod tests {

    use releashd::test_support::integration::settings::NotionPropertyMappingModel;
    use releashd::test_support::integration::settings::NotionRepoConfigModel;

    use releashd::test_support::integration::providers::ProviderKind;
    use releashd::test_support::integration::repository::{
        AppConfigRepository, RepositoryConfigRepository,
    };
    use releashd::test_support::integration::sessions::ProviderExecutable;
    use releashd::test_support::integration::settings::load_or_create_config;
    use releashd::test_support::integration::settings::read_config_if_exists;
    use releashd::test_support::integration::settings::write_config;
    use releashd::test_support::integration::settings::write_config_tmp_file;
    use releashd::test_support::integration::settings::AppConfig;
    use releashd::test_support::integration::settings::ReleashConfig;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn config_path(dir: &TempDir) -> PathBuf {
        dir.path().join("releash.toml")
    }

    #[test]
    pub fn test_repository設定_base_branchの保存と自動への復帰をgitへ永続化する() {
        // Given
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path().join("repo")).unwrap();
        let repo_path = repo.workdir().unwrap().to_str().unwrap();
        let path = config_path(&dir);
        let app_config = AppConfig::new(ReleashConfig::default(), path.clone());

        for base in [Some("develop"), None, None] {
            // When
            let mut config = RepositoryConfigRepository::load(&app_config, repo_path).unwrap();
            config.set_base_branch(base);
            RepositoryConfigRepository::save(&app_config, config).unwrap();

            // Then
            assert_eq!(
                RepositoryConfigRepository::load(&app_config, repo_path)
                    .unwrap()
                    .base_branch(),
                base
            );
            let stored = repo.config().unwrap().get_string("releash.base");
            match base {
                Some(base) => assert_eq!(stored.unwrap(), base),
                None => assert_eq!(stored.unwrap_err().code(), git2::ErrorCode::NotFound),
            }
        }
    }

    #[test]
    pub fn test_repository設定_toml保存失敗時はnotionとbase_branchを保持する() {
        for previous_base in [None, Some("main")] {
            // Given
            let dir = TempDir::new().unwrap();
            let repo = git2::Repository::init(dir.path().join("repo")).unwrap();
            let repo_path = repo.workdir().unwrap().to_str().unwrap();
            if let Some(base) = previous_base {
                repo.config()
                    .unwrap()
                    .set_str("releash.base", base)
                    .unwrap();
            }
            let path = config_path(&dir);
            let mut config = ReleashConfig::default();
            config.notion.insert(
                repo_path.into(),
                NotionRepoConfigModel {
                    api_token: "before-token".into(),
                    database_id: "before-db".into(),
                    property_mapping: Default::default(),
                },
            );
            write_config(&path, &config).unwrap();
            let original = fs::read_to_string(&path).unwrap();
            let app_config = AppConfig::new(config, path.clone());
            let before = RepositoryConfigRepository::load(&app_config, repo_path).unwrap();
            let backup = dir.path().join("saved.toml");
            fs::rename(&path, &backup).unwrap();
            fs::create_dir(&path).unwrap();

            for remove in [false, true] {
                // When
                let mut next = before.clone();
                next.set_base_branch(if previous_base.is_some() {
                    None
                } else {
                    Some("develop")
                });
                next.set_notion(if remove {
                    None
                } else {
                    Some(
                        releashd::test_support::integration::settings::NotionRepoConfig {
                            api_token: "after-token".into(),
                            database_id: "after-db".into(),
                            property_mapping: Default::default(),
                        },
                    )
                });
                let result = RepositoryConfigRepository::save(&app_config, next);

                // Then
                assert!(matches!(result, Err(super::AppConfigError::Technical(_))));
                assert_eq!(
                    RepositoryConfigRepository::load(&app_config, repo_path).unwrap(),
                    before
                );
                assert_eq!(fs::read_to_string(&backup).unwrap(), original);
            }
        }
    }

    #[test]
    pub fn test_repository設定_git保存失敗時はnotionとbase_branchを保持する() {
        // Given
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path().join("repo")).unwrap();
        let repo_path = repo.workdir().unwrap().to_str().unwrap();
        let path = config_path(&dir);
        let config = ReleashConfig::default();
        write_config(&path, &config).unwrap();
        let original = fs::read_to_string(&path).unwrap();
        let app_config = AppConfig::new(config, path.clone());
        let before = RepositoryConfigRepository::load(&app_config, repo_path).unwrap();
        fs::write(repo.path().join("config.lock"), "locked").unwrap();
        let mut next = before.clone();
        next.set_base_branch(Some("develop"));
        next.set_notion(Some(
            releashd::test_support::integration::settings::NotionRepoConfig {
                api_token: "token".into(),
                database_id: "db".into(),
                property_mapping: Default::default(),
            },
        ));

        // When
        let result = RepositoryConfigRepository::save(&app_config, next);

        // Then
        assert!(matches!(result, Err(super::AppConfigError::Technical(_))));
        assert_eq!(
            RepositoryConfigRepository::load(&app_config, repo_path).unwrap(),
            before
        );
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    pub fn test_repository設定_存在しないrepositoryの読み込み失敗を返す() {
        // Given
        let dir = TempDir::new().unwrap();
        let app_config = AppConfig::new(ReleashConfig::default(), config_path(&dir));
        let path = dir.path().join("missing");

        // When
        let result = RepositoryConfigRepository::load(&app_config, path.to_str().unwrap());

        // Then
        assert!(matches!(result, Err(super::AppConfigError::Technical(_))));
    }

    #[test]
    pub fn provider_executable_config既存tomlのoverrideをupdateしresetする() {
        // Given
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        fs::write(&path, "[agents.claude]\ncli_path = '/before/claude'\n[agents.codex]\ncli_path = '/before/codex'\n[app]\nauto_launch = true\n").unwrap();
        let app_config = AppConfig::new(load_or_create_config(&path).unwrap(), path.clone());
        for (provider, other) in [
            (ProviderKind::Claude, ProviderKind::Codex),
            (ProviderKind::Codex, ProviderKind::Claude),
        ] {
            let before = AppConfigRepository::load(&app_config).unwrap();
            let other_executable = before.provider_executable(other).cloned();
            let executable = ProviderExecutable::new("/opt/custom/provider").unwrap();
            let revision = before.provider_configuration_revision(provider);

            // When
            let mut next = before;
            next.update_provider_executable(provider, Some(executable.clone()))
                .unwrap();
            AppConfigRepository::save(&app_config, next).unwrap();

            // Then
            let stored = AppConfigRepository::load(&app_config).unwrap();
            assert_eq!(stored.provider_executable(provider), Some(&executable));
            assert_eq!(
                stored.provider_configuration_revision(provider),
                revision + 1
            );
            assert_eq!(stored.provider_executable(other), other_executable.as_ref());
            let reloaded = super::config_to_domain(&load_or_create_config(&path).unwrap());
            assert_eq!(reloaded.provider_executable(provider), Some(&executable));
            assert!(reloaded.auto_launch());

            // When
            let mut next = stored;
            next.update_provider_executable(provider, None).unwrap();
            AppConfigRepository::save(&app_config, next).unwrap();

            // Then
            let stored = AppConfigRepository::load(&app_config).unwrap();
            assert_eq!(stored.provider_executable(provider), None);
            assert_eq!(
                stored.provider_configuration_revision(provider),
                revision + 2
            );
            assert_eq!(stored.provider_executable(other), other_executable.as_ref());
            assert_eq!(
                super::config_to_domain(&load_or_create_config(&path).unwrap())
                    .provider_executable(provider),
                None
            );
        }
    }

    #[test]
    pub fn provider_executable_config保存失敗時はmemory上のoverrideも進めない() {
        // Given
        let dir = TempDir::new().unwrap();
        let mut config = ReleashConfig::default();
        config.agents.claude.cli_path = Some("/before/claude".into());
        let blocked_parent = dir.path().join("blocked-parent");
        fs::write(&blocked_parent, "not a directory").unwrap();
        let app_config = AppConfig::new(config, blocked_parent.join("releash.toml"));
        let before = AppConfigRepository::load(&app_config).unwrap();

        // When / Then
        for executable in [
            Some(ProviderExecutable::new("/after/claude").unwrap()),
            None,
        ] {
            let mut next = before.clone();
            next.update_provider_executable(ProviderKind::Claude, executable)
                .unwrap();
            next.update_auto_launch(true);
            assert_eq!(
                AppConfigRepository::save(&app_config, next).unwrap_err(),
                super::AppConfigError::Technical(
                    releashd::test_support::integration::platform::TechnicalFailure {
                        nature: releashd::test_support::integration::platform::TechnicalFailureNature::Other,
                        message: format!(
                            "ディレクトリ作成失敗: {}",
                            std::io::Error::from_raw_os_error(17)
                        ),
                    }
                )
            );
            assert_eq!(AppConfigRepository::load(&app_config).unwrap(), before);
        }
    }

    #[test]
    pub fn test_設定作成_serverを出力せず既定値を保存する() {
        // Given
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        // When
        let config = load_or_create_config(&path).unwrap();
        // Then
        assert!(config.telemetry.performance_telemetry);
        assert!(path.exists());
        assert!(!fs::read_to_string(path).unwrap().contains("[server]"));
    }

    #[test]
    pub fn test_旧server設定_読み込み可能で再保存時に削除する() {
        // Given
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        fs::write(
            &path,
            r#"
[server]
bind = "0.0.0.0"
port = 8080
token = "legacy-secret"
mcp_port = 19801
[server.tls]
enabled = true
cert = "cert.pem"
key = "key.pem"
[app]
close_to_tray = false
"#,
        )
        .unwrap();
        let original = fs::read_to_string(&path).unwrap();
        // When
        let config = load_or_create_config(&path).unwrap();
        // Then
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        // When
        write_config(&path, &config).unwrap();
        // Then
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("[server"));
        assert!(!saved.contains("legacy-secret"));
        assert!(!saved.contains("cert.pem"));
        let reloaded = load_or_create_config(&path).unwrap();
        assert!(reloaded.telemetry.performance_telemetry);
    }

    #[test]
    pub fn performance_telemetry_defaults_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 9700\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn performance_telemetry_disabled_persists() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 9700\n\n[telemetry]\nperformance_telemetry = false\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn legacy_telemetry_enabled_false_migrates_to_performance_telemetry_false() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(!config.telemetry.performance_telemetry);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("telemetry_enabled"));
        assert!(saved.contains("performance_telemetry = false"));
    }

    #[test]
    pub fn legacy_telemetry_enabled_true_defaults_performance_telemetry_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = true

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn missing_legacy_telemetry_enabled_defaults_performance_telemetry_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn explicit_new_performance_telemetry_true_wins_over_legacy_false() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"

[telemetry]
performance_telemetry = true
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn explicit_new_performance_telemetry_false_wins_over_legacy_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = true

[server]
token = "existing_token_value_here_with_enough_length_!!"

[telemetry]
performance_telemetry = false
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(!config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn read_config_if_exists_does_not_migrate_legacy_telemetry_enabled() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = read_config_if_exists(&path).unwrap().unwrap();
        let unchanged = fs::read_to_string(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
        assert!(unchanged.contains("telemetry_enabled = false"));
        assert!(!unchanged.contains("performance_telemetry = false"));
    }

    #[test]
    pub fn atomic_write_leaves_no_tmp_file() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let config = ReleashConfig::default();
        write_config(&path, &config).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
        let tmp_files = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(tmp_files, 0);
    }

    #[cfg(unix)]
    #[test]
    pub fn config_tmp_file_is_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let tmp_path = dir.path().join("releash.toml.tmp");

        write_config_tmp_file(&tmp_path, "secret").unwrap();

        let mode = fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    pub fn test_設定保存_既存ファイルの公開権限を引き継がない() {
        use std::os::unix::fs::PermissionsExt;

        // Given
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        // When
        write_config(&path, &ReleashConfig::default()).unwrap();

        // Then
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    pub fn telemetry_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.telemetry.crash_reporting = false;
        config.telemetry.performance_telemetry = false;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(!reloaded.telemetry.crash_reporting);
        assert!(!reloaded.telemetry.performance_telemetry);
    }

    #[test]
    pub fn existing_config_without_telemetry_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.telemetry.crash_reporting);
        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    pub fn workflow_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.workflow.approval_auto_approve = true;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(reloaded.workflow.approval_auto_approve);
    }

    #[test]
    pub fn existing_config_without_workflow_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.workflow.approval_auto_approve);
    }

    #[test]
    pub fn app_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.app.auto_launch = true;
        config.app.start_minimized = true;
        config.app.last_repo_paths = vec!["/repo/path".to_string(), "/repo/path2".to_string()];
        config.app.external_editor = "/Applications/Cursor.app".to_string();
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(reloaded.app.auto_launch);
        assert!(reloaded.app.start_minimized);
        assert_eq!(
            reloaded.app.last_repo_paths,
            vec!["/repo/path", "/repo/path2"]
        );
        assert_eq!(reloaded.app.external_editor, "/Applications/Cursor.app");
    }

    #[test]
    pub fn existing_config_without_app_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.app.auto_launch);
        assert!(!config.app.start_minimized);
        assert!(config.app.last_repo_paths.is_empty());
    }

    #[test]
    pub fn old_config_without_last_repo_paths_gets_empty_default_and_ignores_last_bind_ip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"

[app]
close_to_tray = false
last_root_path = "/old/single/repo"
auto_launch = true
start_minimized = true
external_editor = "cursor"
last_bind_ip = "192.168.1.1"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.app.last_repo_paths.is_empty());
        assert!(config.app.auto_launch);
        assert!(config.app.start_minimized);
        assert_eq!(config.app.external_editor, "cursor");
        let observed = read_config_if_exists(&path).unwrap().unwrap();
        assert_eq!(observed.app, config.app);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    pub fn last_repo_paths_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.app.last_repo_paths = vec![
            "/repo/a".to_string(),
            "/repo/b".to_string(),
            "/repo/c".to_string(),
        ];
        write_config(&path, &config).unwrap();

        let reloaded = load_or_create_config(&path).unwrap();
        assert_eq!(
            reloaded.app.last_repo_paths,
            vec!["/repo/a", "/repo/b", "/repo/c"]
        );
    }

    #[test]
    pub fn existing_config_without_notion_gets_empty_default() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.notion.is_empty());
    }

    #[test]
    pub fn notion_config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.notion.insert(
            "/path/to/repo".to_string(),
            NotionRepoConfigModel {
                api_token: "ntn_test_token".to_string(),
                database_id: "db-id-456".to_string(),
                property_mapping: NotionPropertyMappingModel::default(),
            },
        );
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(reloaded.notion.len(), 1);
        let repo_config = reloaded.notion.get("/path/to/repo").unwrap();
        assert_eq!(repo_config.api_token, "ntn_test_token");
        assert_eq!(repo_config.database_id, "db-id-456");
        assert_eq!(repo_config.property_mapping.title, "Name");
    }

    #[test]
    pub fn notion_config_repository_upsert_get_remove_preserves_all_fields() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let app_config = AppConfig::new(ReleashConfig::default(), path.clone());
        let repo_dir = dir.path().join("repo");
        git2::Repository::init(&repo_dir).unwrap();
        let repo_path = repo_dir.to_str().unwrap();
        let config = releashd::test_support::integration::settings::NotionRepoConfig {
            api_token: "ntn_test_token".to_string(),
            database_id: "db-id-456".to_string(),
            property_mapping:
                releashd::test_support::integration::settings::NotionPropertyMapping {
                    title: "Task Name".to_string(),
                    labels: vec![
                        releashd::test_support::integration::settings::NotionLabelProperty {
                            name: "Status".to_string(),
                            property_type: "status".to_string(),
                        },
                        releashd::test_support::integration::settings::NotionLabelProperty {
                            name: "Tags".to_string(),
                            property_type: "multi_select".to_string(),
                        },
                    ],
                    branch_name: "Branch".to_string(),
                    branch_prefix: "feat/".to_string(),
                },
        };

        // Given
        let mut entity = RepositoryConfigRepository::load(&app_config, repo_path).unwrap();
        assert!(entity.notion().is_none());
        entity.set_notion(Some(config.clone()));
        entity.set_base_branch(Some("develop"));

        // When
        RepositoryConfigRepository::save(&app_config, entity).unwrap();

        // Then
        let stored = RepositoryConfigRepository::load(&app_config, repo_path).unwrap();
        assert_eq!(stored.notion(), Some(&config));
        assert_eq!(stored.base_branch(), Some("develop"));
        let reopened = AppConfig::new(load_or_create_config(&path).unwrap(), path.clone());
        assert_eq!(
            RepositoryConfigRepository::load(&reopened, repo_path).unwrap(),
            stored
        );
        assert_eq!(
            load_or_create_config(&path).unwrap().notion[repo_path].api_token,
            config.api_token
        );

        // When
        let mut entity = stored;
        entity.set_notion(None);
        RepositoryConfigRepository::save(&app_config, entity).unwrap();

        // Then
        assert!(RepositoryConfigRepository::load(&app_config, repo_path)
            .unwrap()
            .notion()
            .is_none());
        assert!(load_or_create_config(&path).unwrap().notion.is_empty());
        assert_eq!(
            RepositoryConfigRepository::load(&app_config, repo_path)
                .unwrap()
                .base_branch(),
            Some("develop")
        );
        assert!(repo_dir.exists());
    }

    #[test]
    pub fn agents_codex_cli_path_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.agents.codex.cli_path = Some("/opt/bin/codex".to_string());
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(
            reloaded.agents.codex.cli_path,
            Some("/opt/bin/codex".to_string())
        );
    }

    #[test]
    pub fn agents_claude_cli_path_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.agents.claude.cli_path = Some("/opt/bin/claude".to_string());
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(
            reloaded.agents.claude.cli_path,
            Some("/opt/bin/claude".to_string())
        );
    }

    #[test]
    pub fn existing_config_without_agents_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.agents.codex.cli_path.is_none());
    }
}
