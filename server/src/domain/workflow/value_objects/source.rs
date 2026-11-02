use std::path::PathBuf;

/// 定義または Facet の実際の取得元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowSource {
    Builtin,
    File { path: PathBuf },
}

impl WorkflowSource {
    /// 取得元が組み込みであるかを返す。
    pub fn is_builtin(&self) -> bool {
        matches!(self, Self::Builtin)
    }
}

#[cfg(test)]
#[path = "source_test.rs"]
mod source_tests;
