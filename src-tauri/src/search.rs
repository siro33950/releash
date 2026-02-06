use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line_number: usize,
    pub line_content: String,
    pub match_start: usize,
    pub match_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub total_matches: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionLocation {
    pub path: String,
    pub line_number: usize,
    pub column: usize,
    pub line_content: String,
    pub kind: String,
}

#[tauri::command]
pub fn search_files(
    root_path: String,
    pattern: String,
    case_sensitive: Option<bool>,
    is_regex: Option<bool>,
    max_results: Option<usize>,
) -> Result<SearchResult, String> {
    let case_sensitive = case_sensitive.unwrap_or(false);
    let is_regex = is_regex.unwrap_or(false);
    let max_results = max_results.unwrap_or(1000);

    let regex_pattern = if is_regex {
        if case_sensitive {
            pattern.clone()
        } else {
            format!("(?i){}", pattern)
        }
    } else {
        let escaped = regex::escape(&pattern);
        if case_sensitive {
            escaped
        } else {
            format!("(?i){}", escaped)
        }
    };

    let re = Regex::new(&regex_pattern).map_err(|e| format!("Invalid pattern: {}", e))?;

    let root = Path::new(&root_path);
    let mut matches = Vec::new();
    let mut total_matches: usize = 0;
    let mut truncated = false;

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            for m in re.find_iter(line) {
                total_matches += 1;
                if matches.len() < max_results {
                    matches.push(SearchMatch {
                        path: relative.clone(),
                        line_number: line_idx + 1,
                        line_content: line.to_string(),
                        match_start: m.start(),
                        match_end: m.end(),
                    });
                } else {
                    truncated = true;
                }
            }
        }
    }

    Ok(SearchResult {
        matches,
        total_matches,
        truncated,
    })
}

fn get_definition_patterns(language: &str) -> Vec<(&'static str, String)> {
    match language {
        "typescript" | "typescriptreact" | "javascript" | "javascriptreact" => {
            vec![
                ("function", r"\b(?:export\s+)?(?:async\s+)?function\s+{symbol}\b".to_string()),
                ("variable", r"\b(?:export\s+)?(?:const|let|var)\s+{symbol}\b".to_string()),
                ("class", r"\b(?:export\s+)?class\s+{symbol}\b".to_string()),
                ("interface", r"\b(?:export\s+)?interface\s+{symbol}\b".to_string()),
                ("type", r"\b(?:export\s+)?type\s+{symbol}\b".to_string()),
                ("enum", r"\b(?:export\s+)?enum\s+{symbol}\b".to_string()),
            ]
        }
        "rust" => {
            vec![
                ("function", r"\b(?:pub\s+)?(?:async\s+)?fn\s+{symbol}\b".to_string()),
                ("struct", r"\b(?:pub\s+)?struct\s+{symbol}\b".to_string()),
                ("enum", r"\b(?:pub\s+)?enum\s+{symbol}\b".to_string()),
                ("trait", r"\b(?:pub\s+)?trait\s+{symbol}\b".to_string()),
                ("type", r"\b(?:pub\s+)?type\s+{symbol}\b".to_string()),
                ("const", r"\b(?:pub\s+)?const\s+{symbol}\b".to_string()),
                ("mod", r"\b(?:pub\s+)?mod\s+{symbol}\b".to_string()),
            ]
        }
        "python" => {
            vec![
                ("function", r"\bdef\s+{symbol}\b".to_string()),
                ("class", r"\bclass\s+{symbol}\b".to_string()),
            ]
        }
        "go" => {
            vec![
                ("function", r"\bfunc\s+{symbol}\b".to_string()),
                ("type", r"\btype\s+{symbol}\b".to_string()),
                ("var", r"\bvar\s+{symbol}\b".to_string()),
                ("const", r"\bconst\s+{symbol}\b".to_string()),
            ]
        }
        _ => {
            vec![
                ("function", r"\bfunction\s+{symbol}\b".to_string()),
                ("class", r"\bclass\s+{symbol}\b".to_string()),
            ]
        }
    }
}

fn language_extensions(language: &str) -> Vec<&'static str> {
    match language {
        "typescript" => vec!["ts", "tsx"],
        "typescriptreact" => vec!["ts", "tsx"],
        "javascript" => vec!["js", "jsx"],
        "javascriptreact" => vec!["js", "jsx"],
        "rust" => vec!["rs"],
        "python" => vec!["py"],
        "go" => vec!["go"],
        _ => vec![],
    }
}

#[tauri::command]
pub fn find_definition(
    root_path: String,
    symbol: String,
    language: String,
) -> Result<Vec<DefinitionLocation>, String> {
    let escaped_symbol = regex::escape(&symbol);
    let patterns = get_definition_patterns(&language);
    let extensions = language_extensions(&language);

    let mut regexes: Vec<(&str, Regex)> = Vec::new();
    for (kind, pattern_template) in &patterns {
        let pattern = pattern_template.replace("{symbol}", &escaped_symbol);
        let re = Regex::new(&pattern).map_err(|e| format!("Invalid pattern: {}", e))?;
        regexes.push((kind, re));
    }

    let root = Path::new(&root_path);
    let mut results = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        if !extensions.is_empty() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !extensions.contains(&ext) {
                continue;
            }
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            for (kind, re) in &regexes {
                if let Some(m) = re.find(line) {
                    results.push(DefinitionLocation {
                        path: relative.clone(),
                        line_number: line_idx + 1,
                        column: m.start() + 1,
                        line_content: line.to_string(),
                        kind: kind.to_string(),
                    });
                }
            }
        }
    }

    Ok(results)
}

#[tauri::command]
pub fn find_references(
    root_path: String,
    symbol: String,
) -> Result<Vec<SearchMatch>, String> {
    let escaped = regex::escape(&symbol);
    let pattern = format!(r"\b{}\b", escaped);
    let re = Regex::new(&pattern).map_err(|e| format!("Invalid pattern: {}", e))?;

    let root = Path::new(&root_path);
    let mut results = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            for m in re.find_iter(line) {
                results.push(SearchMatch {
                    path: relative.clone(),
                    line_number: line_idx + 1,
                    line_content: line.to_string(),
                    match_start: m.start(),
                    match_end: m.end(),
                });
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("hello.ts"),
            "export function greet(name: string) {\n  return `Hello, ${name}!`;\n}\n\nconst greeting = greet('World');\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("main.ts"),
            "import { greet } from './hello';\n\nconsole.log(greet('Test'));\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("app.rs"),
            "pub fn main() {\n    println!(\"hello\");\n}\n\npub struct App {\n    name: String,\n}\n",
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/index.js"),
            "module.exports = {};\n",
        )
        .unwrap();

        fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        dir
    }

    #[test]
    fn test_search_files_basic() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = search_files(root, "greet".to_string(), None, None, None).unwrap();
        assert!(result.matches.len() >= 3);
        assert!(!result.truncated);
    }

    #[test]
    fn test_search_files_case_insensitive() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files(root, "GREET".to_string(), Some(false), None, None).unwrap();
        assert!(result.matches.len() >= 3);
    }

    #[test]
    fn test_search_files_case_sensitive() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files(root, "GREET".to_string(), Some(true), None, None).unwrap();
        assert_eq!(result.matches.len(), 0);
    }

    #[test]
    fn test_search_files_regex() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files(root, r"greet\(".to_string(), None, Some(true), None).unwrap();
        assert!(result.matches.len() >= 2);
    }

    #[test]
    fn test_search_files_respects_gitignore() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files(root, "module.exports".to_string(), None, None, None).unwrap();
        assert_eq!(result.matches.len(), 0);
    }

    #[test]
    fn test_search_files_max_results() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files(root, "greet".to_string(), None, None, Some(1)).unwrap();
        assert_eq!(result.matches.len(), 1);
        assert!(result.truncated);
        assert!(result.total_matches > 1);
    }

    #[test]
    fn test_find_definition_typescript() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition(root, "greet".to_string(), "typescript".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");
        assert_eq!(result[0].line_number, 1);
        assert!(result[0].path.ends_with("hello.ts"));
    }

    #[test]
    fn test_find_definition_rust() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition(root.clone(), "main".to_string(), "rust".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");

        let result =
            find_definition(root, "App".to_string(), "rust".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "struct");
    }

    #[test]
    fn test_find_references() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = find_references(root, "greet".to_string()).unwrap();
        assert!(result.len() >= 3);
    }
}
