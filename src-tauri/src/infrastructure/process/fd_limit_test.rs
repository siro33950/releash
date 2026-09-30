use super::*;

const FD_LIMIT_TEST_CHILD: &str = "RELEASH_FD_LIMIT_TEST_CHILD";

#[test]
fn test_開けるファイル数の上限_softをhardまで引き上げる() {
    if std::env::var_os(FD_LIMIT_TEST_CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env(FD_LIMIT_TEST_CHILD, "1")
            .arg("fd_limit_tests::test_開けるファイル数の上限_softをhardまで引き上げる")
            .arg("--test-threads=1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    // Given
    let mut launchd_default = current_limit().unwrap();
    let target = target_soft_limit(launchd_default.rlim_max).unwrap();
    launchd_default.rlim_cur = (target - 1).min(256);
    // SAFETY: launchd_default keeps the inherited hard limit and only lowers the soft limit.
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &launchd_default) },
        0
    );
    // When
    raise_open_file_limit().unwrap();
    // Then
    assert_eq!(current_limit().unwrap().rlim_cur, target);
}
