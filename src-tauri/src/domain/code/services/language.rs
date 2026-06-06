//! ファイルパスから Monaco Editor の言語識別子を判定する（純粋ロジック）。

/// Detect the Monaco Editor language identifier from a file path's extension.
pub fn get_language_from_path(file_path: &str) -> String {
    let ext = file_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "rs" => "rust",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "py" => "python",
        "go" => "go",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "md" | "markdown" => "markdown",
        "xml" => "xml",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "php" => "php",
        "lua" => "lua",
        "r" => "r",
        "dart" => "dart",
        _ => "plaintext",
    }
    .to_string()
}

#[cfg(test)]
mod language_service_tests {
    use super::*;

    #[test]
    fn test_言語判定_typescript拡張子() {
        assert_eq!(get_language_from_path("src/index.ts"), "typescript");
        assert_eq!(get_language_from_path("App.tsx"), "typescript");
    }

    #[test]
    fn test_言語判定_javascript拡張子() {
        assert_eq!(get_language_from_path("index.js"), "javascript");
        assert_eq!(get_language_from_path("Component.jsx"), "javascript");
    }

    #[test]
    fn test_言語判定_rust拡張子() {
        assert_eq!(get_language_from_path("main.rs"), "rust");
    }

    #[test]
    fn test_言語判定_yaml拡張子() {
        assert_eq!(get_language_from_path("config.yaml"), "yaml");
        assert_eq!(get_language_from_path("ci.yml"), "yaml");
    }

    #[test]
    fn test_言語判定_拡張子なし() {
        assert_eq!(get_language_from_path("Makefile"), "plaintext");
    }

    #[test]
    fn test_言語判定_大文字小文字非依存() {
        assert_eq!(get_language_from_path("file.RS"), "rust");
        assert_eq!(get_language_from_path("file.JSON"), "json");
    }

    #[test]
    fn test_言語判定_ドットファイルで拡張子なし() {
        assert_eq!(get_language_from_path(".gitignore"), "plaintext");
    }

    #[test]
    fn test_言語判定_深いネストパス() {
        assert_eq!(
            get_language_from_path("src/components/panels/Review.tsx"),
            "typescript"
        );
    }
}
