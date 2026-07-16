use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Policy,
    Knowledge,
    Instruction,
}

impl FacetKind {
    /// ストレージ上のディレクトリ名（複数形）。ファイルシステム経路にのみ使う。
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FacetKey(String);

impl FacetKey {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.starts_with('.')
            || value.contains('/')
            || value.contains('\\')
            || value.contains("..")
        {
            return Err(WorkflowError::validation(format!(
                "invalid facet key: {value}"
            )));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[cfg(test)]
mod facet_tests {
    use super::*;

    #[test]
    fn test_facet_kind_dir_nameは既存ディレクトリ名を返す() {
        assert_eq!(FacetKind::Policy.dir_name(), "policies");
        assert_eq!(FacetKind::Instruction.dir_name(), "instructions");
    }

    #[test]
    fn test_facet_key_path要素を拒否する() {
        assert!(FacetKey::new("coding").is_ok());
        assert!(FacetKey::new("../coding").is_err());
        assert!(FacetKey::new("foo/bar").is_err());
    }
}
