/// ファイル全体の差分の 1 行。行番号は 1 から数える。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    /// 変更前と変更後の両方にある行。
    Context { old: u32, new: u32, text: String },
    /// 変更後だけにある行。
    Addition { new: u32, text: String },
    /// 変更前だけにある行。
    Deletion { old: u32, text: String },
}

impl DiffLine {
    /// 変更前の側の行番号。変更後だけにある行には無い。
    pub fn old_number(&self) -> Option<u32> {
        match self {
            Self::Context { old, .. } | Self::Deletion { old, .. } => Some(*old),
            Self::Addition { .. } => None,
        }
    }

    /// 変更後の側の行番号。変更前だけにある行には無い。
    pub fn new_number(&self) -> Option<u32> {
        match self {
            Self::Context { new, .. } | Self::Addition { new, .. } => Some(*new),
            Self::Deletion { .. } => None,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Context { text, .. }
            | Self::Addition { text, .. }
            | Self::Deletion { text, .. } => text,
        }
    }
}
