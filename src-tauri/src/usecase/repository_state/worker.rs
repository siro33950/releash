/// 走査し直すきっかけ。何が変わったかで、走査する範囲が決まる。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InvalidateReason {
    pub shutdown: bool,
    /// ref・HEAD・worktree の登録が変わった。worktree の並びを読み直す。
    pub refs: bool,
    /// worktree のファイルか index が変わった。変更の状態を読み直す。
    pub files: bool,
}

impl InvalidateReason {
    /// 何が変わったか分からないときは、両方を読み直す。
    pub fn change() -> Self {
        Self {
            shutdown: false,
            refs: true,
            files: true,
        }
    }

    pub fn refs() -> Self {
        Self {
            refs: true,
            ..Self::default()
        }
    }

    pub fn files() -> Self {
        Self {
            files: true,
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
        self.shutdown |= other.shutdown;
        self.refs |= other.refs;
        self.files |= other.files;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_merge_keeps_shutdown() {
        let mut reason = InvalidateReason::change();
        reason.merge(InvalidateReason::shutdown());
        reason.merge(InvalidateReason::change());

        assert!(reason.shutdown);
    }

    #[test]
    fn test_走査のきっかけ_まとめると変わった範囲を合わせる() {
        // Given
        let mut reason = InvalidateReason::files();

        // When
        reason.merge(InvalidateReason::refs());

        // Then
        assert_eq!(reason, InvalidateReason::change());
    }
}
