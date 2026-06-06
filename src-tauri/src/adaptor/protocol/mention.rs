//! mention 参照の転送型。
//!
//! domain VO `MentionReference` は serde 非依存（純粋データ）であり、フロント／永続化が
//! 利用する転送表現（camelCase・行範囲省略）は本 adaptor 型が所有する。`into_domain()` /
//! `from_domain()` で双方向変換し、フィールド名・camelCase・省略表現は移行前と等価に保つ。

use serde::{Deserialize, Serialize};

use crate::domain::code::MentionReference;

/// agent / session / workflow の外部境界で受理／返却する mention 参照の転送表現。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionReferenceInput {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end_line: Option<u32>,
}

impl MentionReferenceInput {
    pub fn into_domain(self) -> MentionReference {
        MentionReference {
            file_path: self.file_path,
            start_line: self.start_line,
            end_line: self.end_line,
        }
    }

    pub fn from_domain(d: MentionReference) -> Self {
        Self {
            file_path: d.file_path,
            start_line: d.start_line,
            end_line: d.end_line,
        }
    }
}

/// `Vec<MentionReferenceInput>` → `Vec<MentionReference>` の変換ヘルパー（境界で利用）。
pub fn into_domain_vec(v: Vec<MentionReferenceInput>) -> Vec<MentionReference> {
    v.into_iter().map(|m| m.into_domain()).collect()
}

#[cfg(test)]
mod mention_protocol_tests {
    //! 転送境界の serde 表現が移行前 domain VO（camelCase / 行範囲省略）と等価であることを固定する。
    use super::*;
    use serde_json::json;

    #[test]
    fn test_camelcase_行範囲省略で出力する() {
        let m = MentionReferenceInput {
            file_path: "src/lib.rs".to_string(),
            start_line: None,
            end_line: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v, json!({"filePath": "src/lib.rs"}));
        assert!(v.get("startLine").is_none());
        assert!(v.get("endLine").is_none());
    }

    #[test]
    fn test_camelcaseを受理しdomainへ変換する() {
        let json = r#"{"filePath":"f.rs","startLine":3,"endLine":5}"#;
        let parsed: MentionReferenceInput = serde_json::from_str(json).unwrap();
        let d = parsed.into_domain();
        assert_eq!(d.file_path, "f.rs");
        assert_eq!(d.start_line, Some(3));
        assert_eq!(d.end_line, Some(5));
    }

    #[test]
    fn test_行範囲省略でも受理する() {
        let json = r#"{"filePath":"f.rs"}"#;
        let parsed: MentionReferenceInput = serde_json::from_str(json).unwrap();
        let d = parsed.into_domain();
        assert_eq!(d.start_line, None);
        assert_eq!(d.end_line, None);
    }

    #[test]
    fn test_into_domain_vec() {
        let v = vec![MentionReferenceInput {
            file_path: "a".to_string(),
            start_line: Some(1),
            end_line: Some(2),
        }];
        let domain = into_domain_vec(v);
        assert_eq!(domain.len(), 1);
        assert_eq!(domain[0].start_line, Some(1));
    }
}
