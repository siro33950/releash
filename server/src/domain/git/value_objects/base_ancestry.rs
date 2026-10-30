/// ブランチの先頭と、merge 先になる base の履歴との関係。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseAncestry {
    /// 先頭が base の履歴に含まれる。
    pub in_base_history: bool,
    /// 先頭が base の first-parent の線上にある（base と同じ、または base がそこから進んだだけ）。
    pub on_base_first_parent: bool,
}

impl BaseAncestry {
    /// merge commit を通って base に取り込まれたブランチを merge 済みとする。
    /// base が先へ進んだだけのブランチは、固有の commit を持たないので merge 済みにしない。
    pub fn is_merged(self) -> bool {
        self.in_base_history && !self.on_base_first_parent
    }
}

#[cfg(test)]
#[path = "base_ancestry_test.rs"]
mod base_ancestry_tests;
