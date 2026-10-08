pub fn quote_path_for_shell(path: &str) -> String {
    if path.chars().any(is_shell_metachar) {
        format!("'{}'", path.replace('\'', "'\\''"))
    } else {
        path.to_string()
    }
}

pub fn join_quoted_paths(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| quote_path_for_shell(path))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_shell_metachar(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\''
            | '"'
            | '\\'
            | '!'
            | '$'
            | '`'
            | '('
            | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | '<'
            | '>'
            | '|'
            | ';'
            | '&'
            | '*'
            | '?'
            | '#'
            | '~'
    )
}

#[cfg(test)]
#[path = "shell_test.rs"]
mod shell_tests;
