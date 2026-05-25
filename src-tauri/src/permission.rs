use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionMode {
    Ask,
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
    pub const ASK: &'static str = "ask";
    pub const EDIT: &'static str = "edit";
    pub const FULL: &'static str = "full";
    pub const LEGACY_READONLY: &'static str = "readonly";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ask => Self::ASK,
            Self::Edit => Self::EDIT,
            Self::Full => Self::FULL,
        }
    }

    pub fn allowed_list() -> &'static str {
        "ask, edit, full"
    }

    pub fn parse(value: &str) -> Result<Self, InvalidPermissionMode> {
        match value {
            Self::ASK => Ok(Self::Ask),
            Self::EDIT => Ok(Self::Edit),
            Self::FULL => Ok(Self::Full),
            // Legacy values from sessions/workflows created before issues-1044.
            Self::LEGACY_READONLY => Ok(Self::Ask),
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

impl Serialize for PermissionMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PermissionMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_modes() {
        assert_eq!(PermissionMode::parse("ask").unwrap(), PermissionMode::Ask);
        assert_eq!(PermissionMode::parse("edit").unwrap(), PermissionMode::Edit);
        assert_eq!(PermissionMode::parse("full").unwrap(), PermissionMode::Full);
    }

    #[test]
    fn parse_legacy_modes_normalizes_to_canonical_modes() {
        assert_eq!(
            PermissionMode::parse("readonly").unwrap(),
            PermissionMode::Ask
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
                err.to_string().contains("ask, edit, full"),
                "error message should include allowed modes for '{invalid}'"
            );
        }
    }

    #[test]
    fn rank_ordering_ask_lt_edit_lt_full() {
        // PartialOrd/Ord は enum 宣言順に沿う（ask < edit < full）。
        assert!(PermissionMode::Ask < PermissionMode::Edit);
        assert!(PermissionMode::Edit < PermissionMode::Full);
        assert!(PermissionMode::Ask < PermissionMode::Full);
    }

    #[test]
    fn serde_canonical_values() {
        assert_eq!(
            serde_json::to_string(&PermissionMode::Ask).unwrap(),
            "\"ask\""
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
        let legacy: PermissionMode = serde_json::from_str("\"readonly\"").unwrap();
        assert_eq!(legacy, PermissionMode::Ask);
    }
}
