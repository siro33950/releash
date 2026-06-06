//! ファイルメンションの文字列処理（純粋ロジック）。
//!
//! fuzzy 一致・XML 属性エスケープ・行範囲の抜粋抽出を提供する。ファイル走査や
//! 読み込み（fs I/O）は gateway 層が担い、本サービスは文字列のみを扱う。

/// Subsequence fuzzy match: all characters in `query` appear in `haystack` in order.
pub fn fuzzy_match(haystack: &str, query: &str) -> bool {
    let mut haystack_chars = haystack.chars();
    for q in query.chars() {
        loop {
            match haystack_chars.next() {
                Some(h) if h == q => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// XML 属性値として安全になるようエスケープする。
pub fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// ファイル excerpt を CDATA セクションで包む。
///
/// `<file>...</file>` へ非エスケープでファイル内容を埋め込むと、敵対的内容に
/// `</file></file_context>` 等が含まれた場合に擬似 XML 構造を脱出して agent LLM への
/// プロンプトインジェクションが成立する。CDATA で包むことでこれを無害化する。
/// excerpt 内に CDATA 終端マーカー `]]>` が含まれる場合は CDATA が途中終了するため、
/// `]]>` → `]]]]><![CDATA[>` へ置換してから包む（標準的なエスケープ手順）。
pub fn wrap_cdata(s: &str) -> String {
    let escaped = s.replace("]]>", "]]]]><![CDATA[>");
    format!("<![CDATA[{escaped}]]>")
}

/// 行範囲（1-origin）に基づきファイル内容から抜粋を抽出する。
/// 範囲指定が無い場合はファイル内容全体を返す。
pub fn extract_excerpt(
    file_content: &str,
    start_line: Option<u32>,
    end_line: Option<u32>,
) -> String {
    match (start_line, end_line) {
        (Some(start), Some(end)) => {
            let lines: Vec<&str> = file_content.lines().collect();
            if start == 0 || end < start {
                String::new()
            } else {
                let s = (start as usize) - 1;
                let e = (end as usize).min(lines.len());
                if s >= lines.len() || s >= e {
                    String::new()
                } else {
                    lines[s..e].join("\n")
                }
            }
        }
        (Some(start), None) => {
            if start == 0 {
                String::new()
            } else {
                let lines: Vec<&str> = file_content.lines().collect();
                let s = (start as usize) - 1;
                lines.get(s).unwrap_or(&"").to_string()
            }
        }
        _ => file_content.to_string(),
    }
}

#[cfg(test)]
mod mention_service_tests {
    use super::*;

    #[test]
    fn test_fuzzy一致_基本() {
        assert!(fuzzy_match("src/main.rs", "main"));
        assert!(fuzzy_match("src/main.rs", "src/m"));
        assert!(fuzzy_match("src/components/button.tsx", "bttn"));
        assert!(fuzzy_match("src/components/button.tsx", "btn.t"));
        assert!(!fuzzy_match("src/main.rs", "xyz"));
    }

    #[test]
    fn test_fuzzy一致_空クエリは全一致() {
        assert!(fuzzy_match("anything.rs", ""));
    }

    #[test]
    fn test_抜粋_範囲指定() {
        let content = "line1\nline2\nline3\nline4\nline5\n";
        assert_eq!(
            extract_excerpt(content, Some(2), Some(4)),
            "line2\nline3\nline4"
        );
    }

    #[test]
    fn test_抜粋_単一行() {
        let content = "line1\nline2\nline3\n";
        assert_eq!(extract_excerpt(content, Some(2), None), "line2");
    }

    #[test]
    fn test_抜粋_範囲なしは全体() {
        let content = "line1\nline2\n";
        assert_eq!(extract_excerpt(content, None, None), "line1\nline2\n");
    }

    #[test]
    fn test_抜粋_不正範囲は空() {
        let content = "line1\nline2\n";
        assert_eq!(extract_excerpt(content, Some(0), Some(2)), "");
        assert_eq!(extract_excerpt(content, Some(3), Some(2)), "");
    }

    #[test]
    fn test_xmlエスケープ() {
        assert_eq!(escape_xml_attr(r#"a&b"c<d>e"#), "a&amp;b&quot;c&lt;d&gt;e");
    }
}
