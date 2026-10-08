use super::*;

#[test]
fn test_メインリポジトリパス正規化_unc_prefix保持() {
    let path = Path::new(r"\\server\share\repo");

    let result = path_to_normalized_repo_string(path).unwrap();

    assert_eq!(result, "//server/share/repo");
}
