use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    Readonly,
    Edit,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPermissionMode {
    pub value: String,
}

impl fmt::Display for InvalidPermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid permission mode: {} (allowed: {})",
            if self.value.is_empty() {
                "(empty)"
            } else {
                self.value.as_str()
            },
            PermissionMode::allowed_list()
        )
    }
}

impl std::error::Error for InvalidPermissionMode {}

impl PermissionMode {
    pub const READONLY: &'static str = "readonly";
    pub const EDIT: &'static str = "edit";
    pub const FULL: &'static str = "full";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Readonly => Self::READONLY,
            Self::Edit => Self::EDIT,
            Self::Full => Self::FULL,
        }
    }

    pub fn allowed_list() -> &'static str {
        "readonly, edit, full"
    }

    pub fn parse(value: &str) -> Result<Self, InvalidPermissionMode> {
        match value {
            Self::READONLY => Ok(Self::Readonly),
            Self::EDIT => Ok(Self::Edit),
            Self::FULL => Ok(Self::Full),
            _ => Err(InvalidPermissionMode {
                value: value.to_string(),
            }),
        }
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_modes() {
        assert_eq!(
            PermissionMode::parse("readonly").unwrap(),
            PermissionMode::Readonly
        );
        assert_eq!(PermissionMode::parse("edit").unwrap(), PermissionMode::Edit);
        assert_eq!(PermissionMode::parse("full").unwrap(), PermissionMode::Full);
    }

    #[test]
    fn parse_invalid_modes_rejected() {
        for invalid in &[
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "readwrite",
            "",
            " readonly",
        ] {
            let err = PermissionMode::parse(invalid).unwrap_err();
            assert!(
                err.to_string().contains("readonly, edit, full"),
                "error message should include allowed modes for '{invalid}'"
            );
        }
    }

    #[test]
    fn rank_ordering_readonly_lt_edit_lt_full() {
        // PartialOrd/Ord は enum 宣言順に沿う（readonly < edit < full）。
        assert!(PermissionMode::Readonly < PermissionMode::Edit);
        assert!(PermissionMode::Edit < PermissionMode::Full);
        assert!(PermissionMode::Readonly < PermissionMode::Full);
    }

    #[test]
    fn serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&PermissionMode::Readonly).unwrap(),
            "\"readonly\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionMode::Edit).unwrap(),
            "\"edit\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionMode::Full).unwrap(),
            "\"full\""
        );
        let back: PermissionMode = serde_json::from_str("\"edit\"").unwrap();
        assert_eq!(back, PermissionMode::Edit);
    }
}
