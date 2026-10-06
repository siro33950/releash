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
#[path = "language_test.rs"]
mod language_tests;
