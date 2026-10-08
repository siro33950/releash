use super::*;
#[test]
fn test_管理者設置_パスをshellとapplescriptで引用し通常ファイルを保護する() {
    // Given
    let target = Path::new("/Applications/Releash's App.app/Contents/MacOS/releash");
    let link = Path::new("/usr/local/bin/releash");
    // When
    let script = build_admin_install_script(target, link).unwrap();
    // Then
    assert!(script.contains("Releash'\\''s App.app"));
    assert!(script
        .contains("if [ -L '/usr/local/bin/releash' ] || [ ! -e '/usr/local/bin/releash' ]; then"));
    assert!(script.ends_with("; else exit 1; fi"));
    assert_eq!(applescript_string(r#"echo "a\b""#), r#""echo \"a\\b\"""#);
    assert!(build_admin_install_script(target, Path::new("/")).is_err());
}
