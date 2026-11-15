use super::{LineSide, Target};
use crate::domain::code::value_objects::DiffLine;

/// Thread の対象の行の前に添える差分の行数。
const LEADING_LINES: usize = 3;

/// Thread の対象の行と、その前の数行を、ファイル全体の差分から切り出した 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptLine {
    pub line: DiffLine,
    /// Thread の対象の行か。
    pub targeted: bool,
}

impl Target {
    /// 対象の行と、その前の `LEADING_LINES` 行を、ファイル全体の差分から切り出す。
    ///
    /// - 側を持たない対象は、変更後の側の行番号として探す。変更後の側に行が無い差分では、変更前の側で探す。
    /// - 終わりの側を持たない対象は、始まりと同じ側で終わりを探す。
    /// - 対象が行を持たないときと、対象の行が差分に無いときは、空を返す。
    pub fn excerpt(&self, diff: &[DiffLine]) -> Vec<ExcerptLine> {
        let Some(start) = self.line_number else {
            return Vec::new();
        };
        let fallback = if diff.iter().any(|line| line.new_number().is_some()) {
            LineSide::Additions
        } else {
            LineSide::Deletions
        };
        let side = self.side.unwrap_or(fallback);
        let position = |side, number| {
            diff.iter().position(|line| match side {
                LineSide::Additions => line.new_number() == Some(number),
                LineSide::Deletions => line.old_number() == Some(number),
            })
        };
        let (Some(first), Some(last)) = (
            position(side, start),
            position(
                self.end_side.unwrap_or(side),
                self.end_line.unwrap_or(start),
            ),
        ) else {
            return Vec::new();
        };
        let (first, last) = (first.min(last), first.max(last));
        let from = first.saturating_sub(LEADING_LINES);
        diff[from..=last]
            .iter()
            .enumerate()
            .map(|(offset, line)| ExcerptLine {
                line: line.clone(),
                targeted: from + offset >= first,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "excerpt_test.rs"]
mod excerpt_tests;
