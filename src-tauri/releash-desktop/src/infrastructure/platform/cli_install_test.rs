use super::*;

#[test]
fn install_cli_symlink_skips_app_translocation_path() {
    let exe = PathBuf::from(
        "/private/var/folders/x/AppTranslocation/abc/Releash.app/Contents/MacOS/releash",
    );
    let link = PathBuf::from("/usr/local/bin/releash");

    let status = install_cli_symlink_with_runner(&exe, &link, |_| {
        panic!("admin runner must not be called");
    })
    .unwrap();

    assert_eq!(status, CliInstallStatus::SkippedTranslocated(exe));
}

#[test]
fn applescript_string_escapes_backslashes_and_quotes() {
    assert_eq!(applescript_string(r#"echo "a\b""#), r#""echo \"a\\b\"""#);
}
