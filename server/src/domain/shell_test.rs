pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn leaves_plain_paths_unquoted() {
        assert_eq!(
            quote_path_for_shell("/usr/local/bin/node"),
            "/usr/local/bin/node"
        );
        assert_eq!(
            quote_path_for_shell("relative/path.txt"),
            "relative/path.txt"
        );
    }

    #[test]
    fn quotes_paths_with_spaces_and_metacharacters() {
        assert_eq!(
            quote_path_for_shell("/Users/me/My Documents/file.txt"),
            "'/Users/me/My Documents/file.txt'"
        );
        assert_eq!(
            quote_path_for_shell("/tmp/file(1).txt"),
            "'/tmp/file(1).txt'"
        );
        assert_eq!(quote_path_for_shell("/tmp/$HOME"), "'/tmp/$HOME'");
        assert_eq!(quote_path_for_shell("/tmp/a&b"), "'/tmp/a&b'");
        assert_eq!(quote_path_for_shell("/tmp/a*"), "'/tmp/a*'");
    }

    #[test]
    fn escapes_single_quotes_inside_quoted_path() {
        assert_eq!(
            quote_path_for_shell("/tmp/it's a file"),
            "'/tmp/it'\\''s a file'"
        );
    }

    #[test]
    fn quotes_paths_with_newline_or_carriage_return() {
        assert_eq!(quote_path_for_shell("a\nb"), "'a\nb'");
        assert_eq!(quote_path_for_shell("a\rb"), "'a\rb'");
        assert_eq!(quote_path_for_shell("reboot\n#x"), "'reboot\n#x'");
    }

    #[test]
    fn joins_quoted_paths_with_spaces() {
        assert_eq!(
            join_quoted_paths(&[
                "/tmp/a.txt".to_string(),
                "/tmp/my file.txt".to_string(),
                "/tmp/b.txt".to_string()
            ]),
            "/tmp/a.txt '/tmp/my file.txt' /tmp/b.txt"
        );
        assert_eq!(join_quoted_paths(&[]), "");
    }
}
