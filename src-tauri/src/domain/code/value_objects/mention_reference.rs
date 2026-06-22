//! mention 参照の値オブジェクト（純粋データ）。
//!
//! DOMAIN.md「フロントの都合を domain に漏らさない」方針に従い、本 VO は serde 非依存。
//! フロント入力の転送表現（camelCase・行範囲省略）は境界層が所有し、session 保存モデルは
//! usecase 側の値型で保持する。

#[derive(Debug, Clone, PartialEq)]
pub struct MentionReference {
    pub file_path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}
