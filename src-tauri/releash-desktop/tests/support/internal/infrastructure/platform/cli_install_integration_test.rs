use releash_desktop::test_support::cli_install::{
    install_cli_symlink_with_runner, CliInstallStatus,
};

#[test]
fn install_cli_symlink_creates_link_directly() {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = tmp.path().join("Releash.app/Contents/MacOS/releash");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, "").unwrap();
    let link = tmp.path().join("bin/releash");

    let mut admin_called = false;
    let status = install_cli_symlink_with_runner(&exe, &link, |_| {
        admin_called = true;
        Ok(())
    })
    .unwrap();

    assert_eq!(status, CliInstallStatus::Installed(link.clone()));
    assert_eq!(std::fs::read_link(&link).unwrap(), exe);
    assert!(!admin_called);
}

#[test]
fn install_cli_symlink_noops_when_link_is_current() {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = tmp.path().join("releash");
    std::fs::write(&exe, "").unwrap();
    let link = tmp.path().join("releash-link");
    std::os::unix::fs::symlink(&exe, &link).unwrap();

    let status = install_cli_symlink_with_runner(&exe, &link, |_| {
        panic!("admin runner must not be called");
    })
    .unwrap();

    assert_eq!(status, CliInstallStatus::AlreadyInstalled(link));
}

#[test]
fn install_cli_symlink_replaces_stale_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    let old_exe = tmp.path().join("old-releash");
    let new_exe = tmp.path().join("new-releash");
    std::fs::write(&old_exe, "").unwrap();
    std::fs::write(&new_exe, "").unwrap();
    let link = tmp.path().join("bin/releash");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_exe, &link).unwrap();

    install_cli_symlink_with_runner(&new_exe, &link, |_| {
        panic!("admin runner must not be called");
    })
    .unwrap();

    assert_eq!(std::fs::read_link(&link).unwrap(), new_exe);
}

#[test]
fn install_cli_symlink_refuses_regular_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = tmp.path().join("releash");
    let link = tmp.path().join("releash-link");
    std::fs::write(&exe, "").unwrap();
    std::fs::write(&link, "user owned command").unwrap();

    let err = install_cli_symlink_with_runner(&exe, &link, |_| {
        panic!("admin runner must not be called for non-symlink path");
    })
    .unwrap_err();

    assert!(err.contains("refusing to overwrite non-symlink"));
    assert_eq!(
        std::fs::read_to_string(&link).unwrap(),
        "user owned command"
    );
}

#[test]
fn install_cli_symlink_falls_back_to_admin_script() {
    let tmp = tempfile::TempDir::new().unwrap();
    let exe = tmp.path().join("Releash's App.app/Contents/MacOS/releash");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, "").unwrap();
    let link = tmp.path().join("missing-parent/releash");

    let status = install_cli_symlink_with_runner(&exe, &link, |script| {
        assert!(script.contains("mkdir -p"));
        assert!(script.contains("'Releash'\\''s App.app"));
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(status, CliInstallStatus::Installed(link));
}
