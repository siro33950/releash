//! mention 参照の値オブジェクト（純粋データ）。
//!
//! DOMAIN.md「フロントの都合を domain に漏らさない」方針に従い、本 VO は serde 非依存。
//! フロント／永続化への転送表現（camelCase・行範囲省略）は adaptor 側
//! [`crate::adaptor::protocol::mention::MentionReferenceInput`] が所有し、`into_domain()` /
//! `from_domain()` で双方向変換する。

#[derive(Debug, Clone, PartialEq)]
pub struct MentionReference {
    pub file_path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}
