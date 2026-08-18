//! Composite scope state inside the execution tree.
//!
//! 合成子（sequence / fanout）の実行インスタンスごとに、進行カーソル・子の
//! 実行カウント・子の Artifact をスコープとして所有する。loop_guard のカウント
//! 範囲と兄弟参照の解決空間はこのスコープに閉じ、同じ部品 sequence を再訪する
//! たび新しいスコープが生まれてカウントはフレッシュになる。

use std::collections::HashMap;

use crate::domain::workflow::value_objects::RuntimeArtifact;

use super::FanoutChildRuntime;

/// アクティブな合成子実行インスタンスのスコープ状態。
///
/// 合成子インスタンスが完了・失敗・中断で確定するとスコープは実行木から
/// 取り除かれる（確定した事実は node_executions とイベントが持つ）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeRuntime {
    /// この合成子インスタンスの NodeExecution id。
    pub node_execution_id: String,
    /// 定義 node 名。
    pub node_name: String,
    /// 親スコープ（合成子インスタンス）の NodeExecution id。root は None。
    pub parent_scope_id: Option<String>,
    /// 起動時に親スコープの解決空間から束縛された input パラメータ値。
    pub parameters: Vec<(String, serde_json::Value)>,
    pub kind: ScopeRuntimeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeRuntimeKind {
    Sequence(SequenceScopeRuntime),
    Fanout(FanoutScopeRuntime),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SequenceScopeRuntime {
    /// スコープ内カーソル: 現在アクティブな子の定義名。
    pub current_child: Option<String>,
    /// スコープ内の子ごとの開始回数（attempt 採番と loop_guard 判定）。
    pub child_counts: HashMap<String, u32>,
    /// スコープ内で子が確定させた Artifact（兄弟参照・output の解決空間）。
    pub artifacts: HashMap<String, RuntimeArtifact>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FanoutScopeRuntime {
    /// 展開に使った items（スコープ生成時に確定。slot の item 復元に使う）。
    pub items: Option<Vec<serde_json::Value>>,
    /// 展開された子スロット（宣言順 = items 行 × children 列）。
    pub children: Vec<FanoutChildRuntime>,
    /// スコープ内の子ごとの開始回数（retry の attempt 採番）。
    pub child_counts: HashMap<String, u32>,
}

impl ScopeRuntime {
    pub fn sequence(&self) -> Option<&SequenceScopeRuntime> {
        match &self.kind {
            ScopeRuntimeKind::Sequence(scope) => Some(scope),
            ScopeRuntimeKind::Fanout(_) => None,
        }
    }

    pub fn sequence_mut(&mut self) -> Option<&mut SequenceScopeRuntime> {
        match &mut self.kind {
            ScopeRuntimeKind::Sequence(scope) => Some(scope),
            ScopeRuntimeKind::Fanout(_) => None,
        }
    }

    pub fn fanout(&self) -> Option<&FanoutScopeRuntime> {
        match &self.kind {
            ScopeRuntimeKind::Fanout(scope) => Some(scope),
            ScopeRuntimeKind::Sequence(_) => None,
        }
    }

    pub fn fanout_mut(&mut self) -> Option<&mut FanoutScopeRuntime> {
        match &mut self.kind {
            ScopeRuntimeKind::Fanout(scope) => Some(scope),
            ScopeRuntimeKind::Sequence(_) => None,
        }
    }

    fn child_counts_mut(&mut self) -> &mut HashMap<String, u32> {
        match &mut self.kind {
            ScopeRuntimeKind::Sequence(scope) => &mut scope.child_counts,
            ScopeRuntimeKind::Fanout(scope) => &mut scope.child_counts,
        }
    }

    /// 子の開始を記録し、その attempt（スコープ内 1 始まり）を返す。
    pub fn record_child_start(&mut self, child_name: &str) -> u32 {
        let count = self
            .child_counts_mut()
            .entry(child_name.to_string())
            .or_insert(0);
        *count += 1;
        *count
    }

    /// 子の attempt カウントを少なくとも `attempt` まで進める（retry / replay 用）。
    pub fn raise_child_count_to(&mut self, child_name: &str, attempt: u32) {
        self.child_counts_mut()
            .entry(child_name.to_string())
            .and_modify(|current| *current = (*current).max(attempt))
            .or_insert(attempt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence_scope() -> ScopeRuntime {
        ScopeRuntime {
            node_execution_id: "scope-1".to_string(),
            node_name: "part".to_string(),
            parent_scope_id: None,
            parameters: Vec::new(),
            kind: ScopeRuntimeKind::Sequence(SequenceScopeRuntime::default()),
        }
    }

    #[test]
    fn record_child_start_counts_from_one_and_increases_monotonically() {
        let mut scope = sequence_scope();

        assert_eq!(scope.record_child_start("fix"), 1);
        assert_eq!(scope.record_child_start("fix"), 2);
        assert_eq!(scope.record_child_start("exit"), 1);
        assert_eq!(scope.record_child_start("fix"), 3);
    }

    #[test]
    fn raise_child_count_to_never_lowers_the_count() {
        let mut scope = sequence_scope();
        scope.raise_child_count_to("fix", 3);
        assert_eq!(scope.record_child_start("fix"), 4);

        scope.raise_child_count_to("fix", 2);
        assert_eq!(scope.record_child_start("fix"), 5);
    }
}
