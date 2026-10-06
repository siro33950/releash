pub(crate) mod tests {

    use releash_lib::test_support::integration::process::display_cwd;
    use releash_lib::test_support::integration::process::spawn_shell_command;
    use releash_lib::test_support::integration::process::CommandRunnerError;
    use releash_lib::test_support::integration::process::OutputLimit;
    use tempfile::TempDir;

    const TEST_LABEL: &str = "workflow command";
    const TEST_LIMIT: OutputLimit = OutputLimit {
        max_bytes: 100 * 1024,
        truncation_marker: "... (truncated)",
    };

    #[tokio::test]
    pub async fn shell_command_runs_in_cwd_and_captures_output_and_status() {
        let cwd = TempDir::new().unwrap();
        let canonical_cwd = std::fs::canonicalize(cwd.path()).unwrap();

        let output = spawn_shell_command(
            cwd.path(),
            "printf '%s' \"$PWD\"; printf '%s' err >&2; exit 7",
            std::iter::empty::<(String, String)>(),
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 7);
        assert_eq!(output.stdout, canonical_cwd.to_string_lossy());
        assert_eq!(output.stderr, "err");
        assert!(output.duration_ms < 60_000);
    }

    #[tokio::test]
    pub async fn running_command_label_does_not_retain_shell_command() {
        let cwd = TempDir::new().unwrap();
        let secret_command = "printf '%s' label-secret-sentinel";

        let running = spawn_shell_command(
            cwd.path(),
            secret_command,
            std::iter::empty::<(String, String)>(),
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap();

        assert_eq!(
            running.test_label(),
            format!("workflow command in {}", display_cwd(cwd.path()))
        );
        assert!(!running.test_label().contains(secret_command));
        assert!(!running.test_label().contains("label-secret-sentinel"));

        running.wait().await.unwrap();
    }

    #[tokio::test]
    pub async fn shell_command_cancellation_returns_cancelled() {
        let cwd = TempDir::new().unwrap();
        let running = spawn_shell_command(
            cwd.path(),
            "sleep 30",
            std::iter::empty::<(String, String)>(),
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap();
        let handle = running.handle();

        let waiter = tokio::spawn(async move { running.wait().await });
        handle.request_shutdown();
        let err = waiter.await.unwrap().unwrap_err();

        assert!(matches!(err, CommandRunnerError::Cancelled));
    }

    #[tokio::test]
    pub async fn shell_command_output_capture_is_bounded_and_drains_to_exit() {
        let cwd = TempDir::new().unwrap();
        let output = spawn_shell_command(
            cwd.path(),
            "head -c 200000 /dev/zero | tr '\\0' x; head -c 200000 /dev/zero | tr '\\0' e >&2",
            std::iter::empty::<(String, String)>(),
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 0);
        let marker = TEST_LIMIT.truncation_marker;
        assert!(output.stdout.ends_with(marker));
        assert!(output.stderr.ends_with(marker));
        assert!(output.stdout.len() <= TEST_LIMIT.max_bytes + marker.len());
        assert!(output.stderr.len() <= TEST_LIMIT.max_bytes + marker.len());
    }

    #[tokio::test]
    pub async fn test_shell環境変数_引用付き参照は値を再解釈せず元の内容を渡す() {
        // Given
        let cwd = TempDir::new().unwrap();
        let marker = cwd.path().join("must-not-exist");
        let value = format!(
            "single' double\" `touch {}`\n$HOME; touch {}",
            marker.display(),
            marker.display()
        );

        // When
        let output = spawn_shell_command(
            cwd.path(),
            "printf '%s' \"$DOC\"",
            [("DOC".to_string(), value.clone())],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        // Then
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, value);
        assert!(!marker.exists());
    }

    #[tokio::test]
    pub async fn test_shell環境変数_引用なし参照でも値のshell構文はcommandにならない() {
        // Given
        let cwd = TempDir::new().unwrap();
        let marker = cwd.path().join("must-not-exist");
        let value = format!(
            "one two; touch {} `touch {}`",
            marker.display(),
            marker.display()
        );

        // When
        let output = spawn_shell_command(
            cwd.path(),
            "printf '<%s>\\n' $DOC",
            [("DOC".to_string(), value)],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        // Then
        assert_eq!(output.exit_code, 0);
        assert!(!marker.exists());
        assert!(output.stdout.contains("<two;>"));
        assert!(output.stdout.contains("<`touch>"));
    }

    #[test]
    pub fn test_shell環境変数_nulを含む値は既存spawn_errorになる() {
        let cwd = TempDir::new().unwrap();

        let error = spawn_shell_command(
            cwd.path(),
            "true",
            [("DOC".to_string(), "before\0after".to_string())],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .err()
        .expect("NULを含む環境変数ではprocessを起動できない");

        assert!(matches!(error, CommandRunnerError::Spawn(_)));
    }

    #[test]
    pub fn test_shell環境変数_platform上限超過は既存spawn_errorになる() {
        let cwd = TempDir::new().unwrap();
        let value = "x".repeat(2 * 1024 * 1024);

        let error = spawn_shell_command(
            cwd.path(),
            "true",
            [("DOC".to_string(), value)],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .err()
        .expect("platform上限を超える環境変数ではprocessを起動できない");

        assert!(matches!(error, CommandRunnerError::Spawn(_)));
    }
}
