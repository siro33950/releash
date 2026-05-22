use serde::{Deserialize, Serialize};

/// ワークフローテンプレート定義（[02] Normalized Workflow）。
///
/// 旧 `steps:` 記法は廃止され、`nodes:` 配下の `NodeDefinition` 列が
/// YAML deserialize 先となる。実行インスタンス（`WorkflowRun` / `NodeExecution`）
/// とは語彙が分離される（後者は [03][04] で導入予定）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    pub nodes: Vec<NodeDefinition>,
}

/// facet 参照解決の最大 node 数（DoS 防御の上限）。
///
/// `parallel_children` の親 1 + 子複数を持つ通常的な workflow は
/// 数十 node 程度が現実的な上限であり、超過するファイルは load 段階で拒否する。
pub const MAX_NODES_PER_WORKFLOW: usize = 256;

/// 並列 node の最大子 node 数（DoS 防御の上限）。
pub const MAX_PARALLEL_CHILDREN: usize = 64;

/// load 時に解決した facet コンテンツのキャッシュ。
///
/// `policy` / `knowledge` / `instruction` / `output_contract` / `input_contracts` の
/// キー文字列ではなく、既にファイルから読み込んだ markdown 本文を保持する。実行時には
/// `engine.rs` がこのキャッシュから直接 system_prompt / user_message を組み立てる。
///
/// `#[serde(skip)]` により YAML / JSON のシリアライズ対象外。
/// 永続化形式には未解決の facet 参照キーのみが残り、解決結果は load 経路でのみ生成される。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedFacets {
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    /// 出力側 Contract（step が生成する `<workflow_output>` のデータ仕様）。
    /// 旧 `output_contract` ファセットの本文に相当する。
    pub output_contract: Option<String>,
    /// 入力側 Contract の解決済み本文一覧（[02] Contract 双方向対称性）。
    /// 同一 Contract facet を input / output 双方向で参照できる。
    pub input_contracts: Vec<String>,
}

impl ResolvedFacets {
    /// 主要 facet が未解決（None / 空）なら true を返す。
    ///
    /// 実行系では「facet 参照が宣言されている (`has_facet_refs()` が true) のに
    /// resolved_facets が空」のとき、load 経路の facet 解決が漏れたとみなして
    /// `InvalidWorkflow` を返すガードに使う。
    pub fn is_empty(&self) -> bool {
        self.policy.is_none()
            && self.knowledge.is_none()
            && self.instruction.is_none()
            && self.output_contract.is_none()
            && self.input_contracts.is_empty()
    }
}

/// node 種別。
///
/// 旧 `mode` (auto/approval/interactive) と `parallel` ブロックの有無を統一し、
/// 一つの enum で表現する。`aggregate` は parallel 種別の振る舞いに集約される。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    #[default]
    Agent,
    Bash,
    Approval,
    Parallel,
}

/// 1 つの実行単位を表す正規化済みの node 定義。
///
/// YAML 上は `type: agent | bash | approval | parallel` で種別を表現し、
/// 同階層に種別ごとの振る舞い設定（prompt 用 facet 参照や parallel_children など）と
/// 共通 metadata（transition rules / cycle guard / overrides）を保持する。
///
/// 概念的なフィールド分類（boundary doc 93-114 行）:
/// - `agent_config` 系（agent / approval 種別で使用）: `policy` / `knowledge` /
///   `instruction` / `output_contract` / `pass_previous_response` /
///   `pass_output_from` / `inline_prompt` / `collect`
/// - `command_config` 系（bash 種別で使用）: `command`
/// - `approval_config` 系（approval 種別で使用）: prompt 系フィールドを agent と共有
/// - `parallel_children` 系（parallel 種別で使用）: `parallel_children` / `aggregate`
/// - 共通: `transition_rules` / `cycle_guard` / `resets_cycle_for` / `model` / `permission`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct NodeDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    // --- agent / approval 系 prompt 設定 ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    /// 入力側 Contract 参照キー一覧。
    /// 前段ステップ出力 / task / workflow_variables 等から受け取る入力の
    /// データ仕様を宣言する（[02] Contract 双方向対称性）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contracts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<CollectConfig>,
    // --- bash 系 ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    // --- parallel 系 ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_children: Option<Vec<ChildNodeDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<ParallelAggregate>,
    // --- 共通 ---
    #[serde(default, rename = "rules", skip_serializing_if = "Vec::is_empty")]
    pub transition_rules: Vec<TransitionRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_guard: Option<CycleGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    /// load 経路で `facet.rs` が解決した facet コンテンツのキャッシュ。
    /// YAML serialize 対象外（[02] 境界: 未解決 ref は schema 層に残さない）。
    #[serde(skip)]
    pub resolved_facets: ResolvedFacets,
}

impl NodeDefinition {
    /// node の prompt 関連 facet 参照
    /// （policy / knowledge / instruction / output_contract / input_contracts）が
    /// いずれか 1 つでも指定されているか。
    pub fn has_facet_refs(&self) -> bool {
        self.policy.is_some()
            || self.knowledge.is_some()
            || self.instruction.is_some()
            || self.output_contract.is_some()
            || self.input_contracts.as_ref().is_some_and(|v| !v.is_empty())
    }

    pub fn is_parallel(&self) -> bool {
        matches!(self.node_type, NodeType::Parallel)
    }
}

/// 並列 node 配下の子 node 定義。
///
/// `parallel_children` を `NodeDefinition` の再帰構造から切り離すために導入された
/// 子専用型（[02] schema 境界）。top-level 専用フィールド
/// （`transition_rules` / `cycle_guard` / `resets_cycle_for` / `collect` /
///  `parallel_children` / `aggregate` / `command`）は型レベルで持たない。
///
/// `node_type` は実装上 `Agent` のみが意味を持ち、validation で他の種別は拒否される。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ChildNodeDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contracts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip)]
    pub resolved_facets: ResolvedFacets,
}

impl ChildNodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.policy.is_some()
            || self.knowledge.is_some()
            || self.instruction.is_some()
            || self.output_contract.is_some()
            || self.input_contracts.as_ref().is_some_and(|v| !v.is_empty())
    }
}

/// parallel node 完了後の集約条件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParallelAggregate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransitionRule {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CycleGuard {
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exhausted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectConfig {
    pub from: Vec<String>,
    pub reduce: ReduceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReduceStrategy {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_node() {
        let yaml = r#"
name: agent-only
description: 単一エージェント
nodes:
  - name: implement
    type: agent
    instruction: implement
    policy: coding
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.name, "agent-only");
        assert_eq!(wf.nodes.len(), 1);
        let node = &wf.nodes[0];
        assert_eq!(node.name, "implement");
        assert_eq!(node.node_type, NodeType::Agent);
        assert_eq!(node.instruction.as_deref(), Some("implement"));
        assert_eq!(node.policy.as_deref(), Some("coding"));
    }

    #[test]
    fn parse_approval_node() {
        let yaml = r#"
name: approval-only
description: 承認ノード
nodes:
  - name: approve
    type: approval
    instruction: approve
    policy: planning
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.node_type, NodeType::Approval);
        assert_eq!(node.instruction.as_deref(), Some("approve"));
        assert_eq!(node.policy.as_deref(), Some("planning"));
    }

    #[test]
    fn parse_bash_node() {
        let yaml = r#"
name: bash-only
description: bash node
nodes:
  - name: build
    type: bash
    command: "cargo build"
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.node_type, NodeType::Bash);
        assert_eq!(node.command.as_deref(), Some("cargo build"));
    }

    #[test]
    fn parse_parallel_node_with_aggregate() {
        let yaml = r#"
name: parallel
description: parallel test
nodes:
  - name: implement
    type: agent
    instruction: implement
  - name: parallel-review
    type: parallel
    parallel_children:
      - name: arch-review
        type: agent
        policy: review
        instruction: architecture-review
      - name: security-review
        type: agent
        policy: review
        instruction: security-review
    aggregate:
      all_match: LGTM
      then: report
      else: implement
  - name: report
    type: agent
    instruction: report
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.nodes.len(), 3);
        let parallel = &wf.nodes[1];
        assert_eq!(parallel.node_type, NodeType::Parallel);
        let children = parallel.parallel_children.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "arch-review");
        assert_eq!(children[0].node_type, NodeType::Agent);
        assert_eq!(children[0].policy.as_deref(), Some("review"));
        let agg = parallel.aggregate.as_ref().unwrap();
        assert_eq!(agg.all_match.as_deref(), Some("LGTM"));
        assert!(agg.any_match.is_none());
        assert_eq!(agg.then, "report");
        assert_eq!(agg.r#else, "implement");
    }

    #[test]
    fn parse_transition_rules_and_cycle_guard() {
        let yaml = r#"
name: cycle-test
description: cycle guard test
nodes:
  - name: fix
    type: agent
    instruction: fix
    rules:
      - match: NEEDS_FIX
        next: review
      - match: LGTM
        next: report
    cycle_guard:
      max_iterations: 3
      on_exhausted: report
  - name: review
    type: agent
    instruction: review
  - name: report
    type: approval
    instruction: report
    resets_cycle_for:
      - fix
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let fix = &wf.nodes[0];
        assert_eq!(fix.transition_rules.len(), 2);
        assert_eq!(fix.transition_rules[0].r#match, "NEEDS_FIX");
        assert_eq!(fix.transition_rules[0].next, "review");
        let guard = fix.cycle_guard.as_ref().unwrap();
        assert_eq!(guard.max_iterations, 3);
        assert_eq!(guard.on_exhausted.as_deref(), Some("report"));
        let report = &wf.nodes[2];
        assert_eq!(report.resets_cycle_for, Some(vec!["fix".to_string()]));
    }

    #[test]
    fn parse_model_and_permission_overrides() {
        let yaml = r#"
name: overrides
description: model/permission test
nodes:
  - name: plan
    type: agent
    instruction: plan
    model: test-model
    permission: edit
  - name: implement
    type: agent
    instruction: implement
    model: gpt-5.5
    permission: readonly
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.nodes[0].model.as_deref(), Some("test-model"));
        assert_eq!(wf.nodes[0].permission.as_deref(), Some("edit"));
        assert_eq!(wf.nodes[1].model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn parse_facet_refs() {
        let yaml = r#"
name: facet-test
description: facet ref保持
nodes:
  - name: implement
    type: agent
    policy: coding
    knowledge: architecture
    instruction: implement
    output_contract: plan-doc
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert!(node.has_facet_refs());
        assert_eq!(node.policy.as_deref(), Some("coding"));
        assert_eq!(node.knowledge.as_deref(), Some("architecture"));
        assert_eq!(node.output_contract.as_deref(), Some("plan-doc"));
    }

    #[test]
    fn parse_collect_config() {
        let yaml = r#"
name: collect-test
description: collect設定
nodes:
  - name: review_a
    type: agent
    instruction: review
    rules:
      - match: LGTM
        next: collect_reviews
      - match: NEEDS_FIX
        next: collect_reviews
  - name: collect_reviews
    type: agent
    collect:
      from:
        - review_a
      reduce: any_needs_fix
    rules:
      - match: NEEDS_FIX
        next: review_a
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let collect_node = &wf.nodes[1];
        let collect = collect_node.collect.as_ref().unwrap();
        assert_eq!(collect.from, vec!["review_a".to_string()]);
        assert_eq!(collect.reduce, ReduceStrategy::AnyNeedsFix);
        assert!(!collect_node.has_facet_refs());
    }

    #[test]
    fn parse_pass_previous_response_and_pass_output_from() {
        let yaml = r#"
name: pass-test
description: pass test
nodes:
  - name: a
    type: agent
    instruction: a
  - name: b
    type: agent
    instruction: b
    pass_previous_response: true
    pass_output_from:
      - a
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.nodes[1].pass_previous_response, Some(true));
        assert_eq!(wf.nodes[1].pass_output_from, Some(vec!["a".to_string()]));
    }

    #[test]
    fn parse_inline_prompt() {
        let yaml = r#"
name: inline-test
description: inline prompt
nodes:
  - name: quick
    type: agent
    inline_prompt: "Do a quick analysis"
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(
            wf.nodes[0].inline_prompt.as_deref(),
            Some("Do a quick analysis")
        );
    }

    #[test]
    fn parse_unknown_type_fails() {
        let yaml = r#"
name: bad
description: bad
nodes:
  - name: x
    type: unknown
"#;
        let result: Result<Workflow, _> = serde_saphyr::from_str(yaml);
        assert!(result.is_err());
    }

    /// [02] schema 境界: 旧 schema の `steps:` トップレベルキーは
    /// `nodes:` 必須の新 schema 上では未知フィールドとして拒否される。
    #[test]
    fn parse_old_steps_yaml_fails() {
        let yaml = r#"
name: legacy
description: legacy steps shape
steps:
  - name: x
    mode: auto
    instruction: x
"#;
        let result: Result<Workflow, _> = serde_saphyr::from_str(yaml);
        assert!(
            result.is_err(),
            "旧 steps: 表現は新 schema として deserialize できない"
        );
    }

    /// [02] schema 境界: 旧 schema の `mode:` フィールドは未知フィールドとして拒否される。
    #[test]
    fn parse_old_mode_field_fails() {
        let yaml = r#"
name: legacy
description: legacy mode field
nodes:
  - name: x
    mode: auto
    instruction: x
"#;
        let result: Result<Workflow, _> = serde_saphyr::from_str(yaml);
        assert!(
            result.is_err(),
            "旧 mode: フィールドは新 schema として deserialize できない"
        );
    }

    /// [02] schema 境界: parallel 子 node に top-level 専用フィールド (rules) を書くと
    /// `ChildNodeDefinition` の deny_unknown_fields により拒否される。
    #[test]
    fn parse_child_with_disallowed_top_level_field_fails() {
        let yaml = r#"
name: bad-child
description: child with rules
nodes:
  - name: parent
    type: parallel
    parallel_children:
      - name: c
        type: agent
        instruction: do
        rules:
          - match: LGTM
            next: parent
    aggregate:
      all_match: LGTM
      then: parent
      else: parent
"#;
        let result: Result<Workflow, _> = serde_saphyr::from_str(yaml);
        assert!(
            result.is_err(),
            "ChildNodeDefinition には top-level 専用フィールドを書けない"
        );
    }

    #[test]
    fn parse_builtin_flag() {
        let yaml = r#"
name: built
description: built workflow
builtin: true
nodes:
  - name: x
    type: agent
    instruction: x
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.builtin);
    }
}
