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

    /// User-authored/current API vocabulary parser.
    ///
    /// `parse` keeps the retired `readonly` session value readable for persisted
    /// agent sessions. New workflow inputs must use this canonical parser so the
    /// compatibility branch cannot leak back into the workflow API or YAML.
    pub fn parse_canonical(value: &str) -> Result<Self, InvalidPermissionMode> {
        match value {
            Self::ASK => Ok(Self::Ask),
            Self::EDIT => Ok(Self::Edit),
            Self::FULL => Ok(Self::Full),
            _ => Err(InvalidPermissionMode {
                value: value.to_string(),
            }),
        }
    }

    pub fn parse(value: &str) -> Result<Self, InvalidPermissionMode> {
        match value {
            // Legacy value from persisted agent sessions created before issues-1044.
            Self::LEGACY_READONLY => Ok(Self::Ask),
            _ => Self::parse_canonical(value),
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
    fn parse_canonical_accepts_only_current_modes() {
        for (value, expected) in [
            ("ask", PermissionMode::Ask),
            ("edit", PermissionMode::Edit),
            ("full", PermissionMode::Full),
        ] {
            assert_eq!(PermissionMode::parse_canonical(value).unwrap(), expected);
        }

        for invalid in ["read", "readonly", "acceptEdits"] {
            let err = PermissionMode::parse_canonical(invalid).unwrap_err();
            assert_eq!(err.value, invalid);
            assert!(err.to_string().contains("allowed: ask, edit, full"));
        }
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
}
