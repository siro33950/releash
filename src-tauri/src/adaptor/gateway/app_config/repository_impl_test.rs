use super::*;
use crate::domain::workflow::secret_masker;
use tempfile::TempDir;

#[test]
fn test_設定の秘匿対象_旧serverとnotionの単独値を出力とartifactで秘匿する() {
    // Given
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("releash.toml");
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
    fs::write(&path, legacy).unwrap();
    let config = AppConfig::new(load_or_create_config(&path).unwrap(), path.clone());

    // When
    let secrets = config.configured_secret_values().unwrap();

    // Then
    for value in ["0123456789abcdef0123456789abcdef", "notion-value-1234"] {
        assert!(secrets.contains(&value.to_string()));
        assert_eq!(secret_masker::mask_sensitive_text(value, &[]), value);
        assert_eq!(
            secret_masker::mask_sensitive_text(value, &secrets),
            "[REDACTED]"
        );
        assert_eq!(
            secret_masker::mask_sensitive_artifact(
                "result",
                serde_json::json!({"output": [value, {"nested": value}], "count": 1}),
                &secrets,
            ),
            serde_json::json!({"output": ["[REDACTED]", {"nested": "[REDACTED]"}], "count": 1}),
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
    let before = ConfigRepository::load(&config).unwrap();
    assert!(!before.app.close_to_tray);
    let notion = NotionConfigRepository::get(&config, "/repo")
        .unwrap()
        .unwrap();
    assert_eq!(notion.database_id, "database-id");

    // When
    ConfigRepository::save(&config, before.clone()).unwrap();

    // Then
    let saved = fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("[server"));
    assert!(!saved.contains("0123456789abcdef0123456789abcdef"));
    assert!(!saved.contains("cert.pem"));
    assert_eq!(ConfigRepository::load(&config).unwrap(), before);
    assert_eq!(
        NotionConfigRepository::get(&config, "/repo").unwrap(),
        Some(notion)
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
fn test_設定の秘匿対象_旧server_tokenの長さと欠損を扱う() {
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
fn test_設定の秘匿対象_ファイル不在でもメモリ上のnotionを収集する() {
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
fn test_設定の秘匿対象_設定ファイル取得失敗でも収集済みnotionを保持する() {
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
fn test_設定の秘匿対象_読み取り失敗を通知する() {
    // Given
    let dir = TempDir::new().unwrap();
    let config = AppConfig::new(ReleashConfig::default(), dir.path().to_path_buf());
    // When
    let error = config.read_legacy_server_token().unwrap_err();
    // Then
    assert!(matches!(error, AppConfigError::Repository(_)));
    assert!(error.to_string().contains("読み込み失敗"));
}

#[test]
fn test_設定の秘匿対象_パース失敗にtokenを含めず通知する() {
    // Given
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("releash.toml");
    fs::write(&path, "[server]\ntoken = 'legacy-sensitive-value' invalid").unwrap();
    let config = AppConfig::new(ReleashConfig::default(), path);
    // When
    let error = config.read_legacy_server_token().unwrap_err();
    // Then
    assert!(matches!(error, AppConfigError::Repository(_)));
    assert!(error.to_string().contains("パース失敗"));
    assert!(!error.to_string().contains("legacy-sensitive-value"));
}
