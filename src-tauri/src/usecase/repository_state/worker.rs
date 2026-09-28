#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvalidateReason {
    pub file_change: bool,
    pub git_change: bool,
    pub branch_change: bool,
    pub shutdown: bool,
    pub path: Option<String>,
}

impl InvalidateReason {
    pub fn initial() -> Self {
        Self {
            git_change: true,
            branch_change: true,
            ..Self::default()
        }
    }

    pub fn file(path: Option<String>) -> Self {
        Self {
            file_change: true,
            path,
            ..Self::default()
        }
    }

    pub fn git(branch_change: bool) -> Self {
        Self {
            git_change: true,
            branch_change,
            ..Self::default()
        }
    }

    pub fn shutdown() -> Self {
        Self {
            shutdown: true,
            ..Self::default()
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.file_change |= other.file_change;
        self.git_change |= other.git_change;
        self.branch_change |= other.branch_change;
        self.shutdown |= other.shutdown;
        if self.path.is_none() {
            self.path = other.path;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_merge_flags_and_keep_first_path() {
        let mut a = InvalidateReason::file(Some("a.txt".to_string()));
        a.merge(InvalidateReason::git(true));
        a.merge(InvalidateReason::file(Some("b.txt".to_string())));

        assert!(a.file_change);
        assert!(a.git_change);
        assert!(a.branch_change);
        assert_eq!(a.path.as_deref(), Some("a.txt"));
    }
}
