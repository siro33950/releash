use crate::adaptor::gateway::workflow::builtin;
use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::facet::{self, FacetKind};
use crate::adaptor::gateway::workflow::prompt_rendering;
use crate::adaptor::gateway::workflow::schema::{NodeDefinition, ReduceStrategy, Rule, Workflow};
use crate::domain::workflow::validation;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const ALL_FACET_KINDS: [FacetKind; 3] = [
    FacetKind::Policy,
    FacetKind::Knowledge,
    FacetKind::Instruction,
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticItem {
    pub severity: Severity,
    pub message: String,
    /// 対象の workflow 名（ファセット診断の場合は None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// 対象の step 名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
    /// 対象のファセットキー（ファセット診断の場合）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_key: Option<String>,
    /// 対象のファセット種別
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_kind: Option<String>,
    /// 対象フィールド
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DiagnosticSummary {
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    /// workflow名 → そのworkflowの診断サマリ
    pub workflow_summaries: HashMap<String, DiagnosticSummary>,
    /// "kind/key" → そのファセットの診断サマリ
    pub facet_summaries: HashMap<String, DiagnosticSummary>,
    /// ファセットキー → 参照元workflow/step情報のリスト
    pub facet_usage: HashMap<String, Vec<FacetUsageEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub step_name: String,
    pub slot: String,
}

/// 全ワークフロー・全ファセットを走査し診断結果を返す
pub fn diagnose_all(workflows_dir: &Path, facets_base_dir: &Path) -> DiagnosticReport {
    let mut items = Vec::new();
    let mut workflow_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_usage: HashMap<String, Vec<FacetUsageEntry>> = HashMap::new();

    // --- 全ファセットキーのセットを構築（参照存在チェック用） ---
    let all_facet_keys = collect_all_facet_keys(facets_base_dir);

    // --- ワークフロー診断 ---
    let workflows = load_all_workflows(workflows_dir, facets_base_dir);
    for (name, wf_result) in &workflows {
        match wf_result {
            Err(msg) => {
                let item = DiagnosticItem {
                    severity: Severity::Error,
                    message: format!("ワークフロー '{name}' の読み込みに失敗: {msg}"),
                    workflow_name: Some(name.clone()),
                    step_name: None,
                    facet_key: None,
                    facet_kind: None,
                    field: None,
                };
                add_diagnostic(&mut items, &mut workflow_summaries, name, item);
            }
            Ok(wf) => {
                diagnose_workflow(
                    wf,
                    &all_facet_keys,
                    &mut items,
                    &mut workflow_summaries,
                    &mut facet_usage,
                );
            }
        }
    }

    // --- ファセット診断 ---
    for kind in &ALL_FACET_KINDS {
        let summaries = facet::list_facet_summaries(*kind, facets_base_dir).unwrap_or_default();
        for summary in &summaries {
            let facet_id = format!("{}/{}", kind.dir_name(), summary.key);

            // ファセットキー命名規則チェック
            if facet::validate_facet_key(&summary.key).is_err() {
                let item = DiagnosticItem {
                    severity: Severity::Error,
                    message: format!(
                        "ファセットキー '{}' が命名規則に違反しています",
                        summary.key
                    ),
                    workflow_name: None,
                    step_name: None,
                    facet_key: Some(summary.key.clone()),
                    facet_kind: Some(kind.dir_name().to_string()),
                    field: Some("key".to_string()),
                };
                add_diagnostic(&mut items, &mut facet_summaries, &facet_id, item);
            }

            // ビルトイン info
            if summary.builtin {
                let item = DiagnosticItem {
                    severity: Severity::Info,
                    message: format!(
                        "ビルトインファセット '{}' ({})",
                        summary.key,
                        kind.dir_name()
                    ),
                    workflow_name: None,
                    step_name: None,
                    facet_key: Some(summary.key.clone()),
                    facet_kind: Some(kind.dir_name().to_string()),
                    field: None,
                };
                add_diagnostic(&mut items, &mut facet_summaries, &facet_id, item);
            }

            // テンプレート変数チェック
            if let Ok(content) = facet::load_facet(*kind, &summary.key, facets_base_dir) {
                check_template_variables(
                    &content,
                    &summary.key,
                    kind.dir_name(),
                    &facet_id,
                    &mut items,
                    &mut facet_summaries,
                );
            }
        }
    }

    DiagnosticReport {
        items,
        workflow_summaries,
        facet_summaries,
        facet_usage,
    }
}

/// 全ファセットキーを収集（"kind/key" 形式）
fn collect_all_facet_keys(base_dir: &Path) -> HashSet<String> {
    let mut keys = HashSet::new();
    for kind in &ALL_FACET_KINDS {
        if let Ok(list) = facet::list_facets(*kind, base_dir) {
            for key in list {
                keys.insert(format!("{}/{}", kind.dir_name(), key));
            }
        }
    }
    keys
}

/// ディスク + builtin のワークフロー一覧を読み込み
fn load_all_workflows(
    dir: &Path,
    facets_base_dir: &Path,
) -> Vec<(String, Result<Workflow, String>)> {
    let mut results = Vec::new();

    // ディスク上のカスタムワークフロー（validate() をスキップし全件走査）
    let mut seen = HashSet::new();
    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("yml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let name = stem.to_string();
                        let result = std::fs::read_to_string(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|content| {
                                let workflow = serde_saphyr::from_str::<Workflow>(&content)
                                    .map_err(|e| e.to_string())?;
                                let _ = facet::resolve_workflow_facets(&workflow, facets_base_dir);
                                Ok(workflow)
                            });
                        seen.insert(name.clone());
                        results.push((name, result));
                    }
                }
            }
        }
    }

    // ビルトインワークフロー
    for summary in builtin::list_builtin_workflows() {
        if !seen.contains(&summary.name) {
            match builtin::load_builtin_workflow_resolved(&summary.name) {
                Ok(Some(wf)) => results.push((summary.name, Ok(wf))),
                Ok(None) => results.push((
                    summary.name.clone(),
                    Err(format!(
                        "ビルトインワークフロー '{}' の読み込みに失敗",
                        summary.name
                    )),
                )),
                Err(err) => results.push((
                    summary.name.clone(),
                    Err(format!(
                        "ビルトインワークフロー '{}' の読み込みに失敗: {err}",
                        summary.name
                    )),
                )),
            }
        }
    }

    results
}

/// validation::validate() のエラーのうち、diagnose_workflow 側で個別にチェック済みのものを判定
fn is_covered_by_diagnostics(e: &validation::ValidationError) -> bool {
    use validation::ValidationError;
    matches!(
        e,
        ValidationError::EmptyName
            | ValidationError::InvalidChars { .. }
            | ValidationError::MissingFacet { .. }
            | ValidationError::UnknownCollectFrom { .. }
    )
}

/// ValidationError からステップ名とフィールド名を抽出
fn validation_error_context(e: &validation::ValidationError) -> (Option<String>, Option<String>) {
    use validation::ValidationError;
    match e {
        ValidationError::EmptyName | ValidationError::InvalidChars { .. } => {
            (None, Some("name".to_string()))
        }
        ValidationError::EmptySteps => (None, Some("steps".to_string())),
        ValidationError::DuplicateStep { name } => (Some(name.clone()), Some("name".to_string())),
        ValidationError::ParallelChildNameConflict { child } => (
            Some(child.clone()),
            Some("parallel_children.name".to_string()),
        ),
        ValidationError::AggregateInvalidConfig { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::AggregateUnknownTarget { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::ParallelChildMissingFacet { parent, .. } => (
            Some(parent.clone()),
            // 旧ラベル "parallel.facets" と同じく、policy/knowledge/instruction/artifact_contract
            // のいずれかの欠落を表す論理グループ名として "facets" を用いる
            // （新 schema の YAML キーは個別だが、`MissingFacet` 側も "facets" 表現）。
            Some("parallel_children.facets".to_string()),
        ),
        ValidationError::UnknownRuleTarget { step, .. }
        | ValidationError::InvalidRules { step, .. } => {
            (Some(step.clone()), Some("rules.next".to_string()))
        }
        ValidationError::MissingFacet { step } => (Some(step.clone()), Some("facets".to_string())),
        ValidationError::UnknownCollectFrom { step, .. } => {
            (Some(step.clone()), Some("collect.from".to_string()))
        }
        ValidationError::InvalidArtifactReference { .. } => (None, Some("inputs".to_string())),
        ValidationError::InvalidPermissionMode { step, .. } => {
            (Some(step.clone()), Some("permission".to_string()))
        }
        ValidationError::MissingPermissionMode { step } => {
            (Some(step.clone()), Some("permission".to_string()))
        }
        ValidationError::UnknownModel { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::InvalidModelFormat { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::ModelResolutionFailed { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::EmptyCommand { step } => (Some(step.clone()), Some("command".to_string())),
        ValidationError::DisallowedFieldForKind { step, field, .. } => {
            (Some(step.clone()), Some(field.to_string()))
        }
        ValidationError::TooManyNodes { .. } => (None, Some("nodes".to_string())),
        ValidationError::TooManyParallelChildren { step, .. } => {
            (Some(step.clone()), Some("parallel_children".to_string()))
        }
        ValidationError::UnknownSchemaRef { step, slot, .. }
        | ValidationError::InvalidSchemaRef { step, slot, .. } => {
            (Some(step.clone()), Some((*slot).to_string()))
        }
        ValidationError::InvalidSchema { .. } => (None, Some("schemas".to_string())),
        ValidationError::InvalidArtifactSchema { step, .. }
        | ValidationError::ReservedArtifactField { step, .. } => {
            (Some(step.clone()), Some("artifact".to_string()))
        }
    }
}

/// ステップが完了後に定義順で暗黙進行する経路はない。
/// rules なしは終端 node として扱う。
fn can_advance_sequentially(step: &NodeDefinition) -> bool {
    let _ = step;
    false
}

/// 到達可能性の計算結果。
struct ReachabilityResult<'a> {
    /// 明示的遷移（rules.next, aggregate.then/else）+ 最初のステップで到達可能
    explicitly_reachable: HashSet<&'a str>,
    /// 明示的遷移 + 暗黙的順次進行を含めた全到達可能
    all_reachable: HashSet<&'a str>,
}

/// 到達可能なステップ名の集合を計算する。
/// 明示的遷移と暗黙的順次進行を区別して返す。
fn compute_reachable_steps<'a>(
    wf: &'a Workflow,
    step_names: &HashSet<&'a str>,
) -> ReachabilityResult<'a> {
    let mut explicitly_reachable: HashSet<&str> = HashSet::new();

    if let Some(first) = wf.nodes.first() {
        explicitly_reachable.insert(&first.name);
    }

    // 明示的遷移先を収集
    for step in &wf.nodes {
        for rule in &step.rules {
            for target in schema_rule_targets(rule) {
                if step_names.contains(target) {
                    explicitly_reachable.insert(target);
                }
            }
        }
        if let Some(agg) = step.fanout().and_then(|fanout| fanout.aggregate.as_ref()) {
            if step_names.contains(agg.then.as_str()) {
                explicitly_reachable.insert(&agg.then);
            }
            if step_names.contains(agg.r#else.as_str()) {
                explicitly_reachable.insert(&agg.r#else);
            }
        }
    }

    // 暗黙的順次進行を伝播（到達可能なステップから次ステップへ）
    let mut all_reachable = explicitly_reachable.clone();
    loop {
        let mut added = false;
        for (i, step) in wf.nodes.iter().enumerate() {
            if !all_reachable.contains(step.name.as_str()) {
                continue;
            }
            if i + 1 >= wf.nodes.len() {
                continue;
            }
            let next = &wf.nodes[i + 1];
            if all_reachable.contains(next.name.as_str()) {
                continue;
            }
            if can_advance_sequentially(step) {
                all_reachable.insert(&next.name);
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    ReachabilityResult {
        explicitly_reachable,
        all_reachable,
    }
}

fn diagnose_workflow(
    wf: &Workflow,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
    let name = &wf.name;

    // バリデーション（validate_all）を実行し、
    // 診断側で個別チェックしていない項目をエラーとして報告
    let workflow = workflow_definition_to_domain(wf);
    for e in validation::validate_all(&workflow) {
        let (step_name, field) = validation_error_context(&e);
        if !is_covered_by_diagnostics(&e) {
            let item = DiagnosticItem {
                severity: Severity::Error,
                message: e.to_string(),
                workflow_name: Some(name.clone()),
                step_name,
                facet_key: None,
                facet_kind: None,
                field,
            };
            add_diagnostic(items, workflow_summaries, name, item);
        }
    }

    // workflow名の命名規則チェック
    if validation::validate_name(name).is_err() {
        let item = DiagnosticItem {
            severity: Severity::Error,
            message: format!("ワークフロー名 '{name}' が命名規則に違反しています"),
            workflow_name: Some(name.clone()),
            step_name: None,
            facet_key: None,
            facet_kind: None,
            field: Some("name".to_string()),
        };
        add_diagnostic(items, workflow_summaries, name, item);
    }

    // ビルトイン info
    if wf.builtin {
        let item = DiagnosticItem {
            severity: Severity::Info,
            message: format!("ビルトインワークフロー '{name}'"),
            workflow_name: Some(name.clone()),
            step_name: None,
            facet_key: None,
            facet_kind: None,
            field: None,
        };
        add_diagnostic(items, workflow_summaries, name, item);
    }

    // step名の集合（到達可能性チェック・遷移先チェック用、トップレベルstep名のみ）
    let step_names: HashSet<&str> = wf.nodes.iter().map(|s| s.name.as_str()).collect();
    // 先行step名の集合（collect.from のチェック用）
    // validation.rs と同じロジック: collect.from は先行ステップのみ参照可能
    let mut preceding_step_names: HashSet<&str> = HashSet::new();
    let reachability = compute_reachable_steps(wf, &step_names);

    // 各stepを診断
    for step in &wf.nodes {
        // collect.from 参照チェック（先行stepのみ参照可能）
        if let Some(ref collect) = step.collect {
            for from in &collect.from {
                if !preceding_step_names.contains(from.as_str()) {
                    let msg = if step_names.contains(from.as_str()) {
                        format!(
                            "ステップ '{}' のcollect.fromがまだ定義されていないステップ '{}' を参照しています（先行ステップのみ参照可能）",
                            step.name, from
                        )
                    } else {
                        format!(
                            "ステップ '{}' のcollect.fromが存在しないステップ '{}' を参照しています",
                            step.name, from
                        )
                    };
                    let item = DiagnosticItem {
                        severity: Severity::Error,
                        message: msg,
                        workflow_name: Some(name.clone()),
                        step_name: Some(step.name.clone()),
                        facet_key: None,
                        facet_kind: None,
                        field: Some("collect.from".to_string()),
                    };
                    add_diagnostic(items, workflow_summaries, name, item);
                }
            }

            // collect元stepにrulesがないwarning
            if matches!(
                collect.reduce,
                ReduceStrategy::AnyNeedsFix | ReduceStrategy::AllPassed
            ) {
                for from in &collect.from {
                    if let Some(source_step) = wf.nodes.iter().find(|s| s.name == *from) {
                        if source_step.rules.is_empty() && !source_step.is_fanout() {
                            let item = DiagnosticItem {
                                severity: Severity::Warning,
                                message: format!(
                                    "collect元ステップ '{}' にrulesが未設定です（{:?}リデュースで結果がNoneになる可能性）",
                                    from, collect.reduce
                                ),
                                workflow_name: Some(name.clone()),
                                step_name: Some(step.name.clone()),
                                facet_key: None,
                                facet_kind: None,
                                field: Some("collect.reduce".to_string()),
                            };
                            add_diagnostic(items, workflow_summaries, name, item);
                        }
                    }
                }
            }
        }

        // ファセット参照の存在チェック + usage 記録
        FacetRefCheckContext::new(name, all_facet_keys, items, workflow_summaries, facet_usage)
            .check_step(
                &step.name,
                &FacetRefs {
                    policy: step
                        .session()
                        .and_then(|session| session.facets.policy.as_deref()),
                    knowledge: step
                        .session()
                        .and_then(|session| session.facets.knowledge.as_deref()),
                    instruction: step
                        .session()
                        .and_then(|session| session.facets.instruction.as_deref()),
                },
            );

        // command/fanout は実行構造を kind block に持つため facet は不要。
        if step.is_session() && step.collect.is_none() && !step.has_facet_refs() {
            let item = DiagnosticItem {
                severity: Severity::Error,
                message: format!("ステップ '{}' にはファセット参照が必要です", step.name),
                workflow_name: Some(name.clone()),
                step_name: Some(step.name.clone()),
                facet_key: None,
                facet_kind: None,
                field: Some("facets".to_string()),
            };
            add_diagnostic(items, workflow_summaries, name, item);
        }

        // parallel block の子step 診断
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                FacetRefCheckContext::new(
                    name,
                    all_facet_keys,
                    items,
                    workflow_summaries,
                    facet_usage,
                )
                .check_step(
                    &child.name,
                    &FacetRefs {
                        policy: child.facets.policy.as_deref(),
                        knowledge: child.facets.knowledge.as_deref(),
                        instruction: child.facets.instruction.as_deref(),
                    },
                );
            }
        }

        // 到達可能性チェック（最初のstepを除く）
        if wf.nodes.first().map(|s| &s.name) != Some(&step.name) {
            let step_str = step.name.as_str();
            if !reachability.all_reachable.contains(step_str) {
                // 明示的にも暗黙的にも到達不能
                let item = DiagnosticItem {
                    severity: Severity::Warning,
                    message: format!(
                        "ステップ '{}' はどこからも遷移されません（到達不能）",
                        step.name
                    ),
                    workflow_name: Some(name.clone()),
                    step_name: Some(step.name.clone()),
                    facet_key: None,
                    facet_kind: None,
                    field: None,
                };
                add_diagnostic(items, workflow_summaries, name, item);
            } else if !reachability.explicitly_reachable.contains(step_str) {
                // 暗黙的順次進行でのみ到達可能 → 明示的遷移を推奨
                let item = DiagnosticItem {
                    severity: Severity::Warning,
                    message: format!(
                        "ステップ '{}' への明示的な遷移が定義されていません（暗黙的な順次進行で到達）",
                        step.name
                    ),
                    workflow_name: Some(name.clone()),
                    step_name: Some(step.name.clone()),
                    facet_key: None,
                    facet_kind: None,
                    field: None,
                };
                add_diagnostic(items, workflow_summaries, name, item);
            }
        }

        // preceding_step_names を更新（validation.rs と同じロジック）
        preceding_step_names.insert(&step.name);
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }
}

fn schema_rule_targets(rule: &Rule) -> Vec<&str> {
    match rule {
        Rule::When { then, next, .. } => vec![then.as_str(), next.as_str()],
        Rule::Switch { cases, next, .. } => cases
            .values()
            .map(String::as_str)
            .chain(next.iter().map(String::as_str))
            .collect(),
        Rule::LoopGuard { on_exhausted, .. } => vec![on_exhausted.as_str()],
        Rule::Next(next) => vec![next.as_str()],
    }
}

struct FacetRefs<'a> {
    policy: Option<&'a str>,
    knowledge: Option<&'a str>,
    instruction: Option<&'a str>,
}

/// 複数の facet 参照を 1 つの step スコープで一括検査するためのコンテキスト。
///
/// 旧 `check_single_facet_ref` は 9 引数で都度 sink / 一覧 / workflow 名を渡していたが、
/// それらは「`diagnose_workflow` の 1 走行を通じて共有される」性質のもの。
/// このコンテキストにまとめて `ctx.check(step, slot, kind, key)` の形で呼び出すことで、
/// 凝集度を上げ `#[allow(clippy::too_many_arguments)]` を不要にする。
struct FacetRefCheckContext<'a> {
    workflow_name: &'a str,
    all_facet_keys: &'a HashSet<String>,
    items: &'a mut Vec<DiagnosticItem>,
    workflow_summaries: &'a mut HashMap<String, DiagnosticSummary>,
    facet_usage: &'a mut HashMap<String, Vec<FacetUsageEntry>>,
}

impl<'a> FacetRefCheckContext<'a> {
    fn new(
        workflow_name: &'a str,
        all_facet_keys: &'a HashSet<String>,
        items: &'a mut Vec<DiagnosticItem>,
        workflow_summaries: &'a mut HashMap<String, DiagnosticSummary>,
        facet_usage: &'a mut HashMap<String, Vec<FacetUsageEntry>>,
    ) -> Self {
        Self {
            workflow_name,
            all_facet_keys,
            items,
            workflow_summaries,
            facet_usage,
        }
    }

    /// 単一の facet 参照について usage 記録と存在チェックを行う。
    fn check(&mut self, step_name: &str, slot: &str, kind: FacetKind, key: &str) {
        let facet_id = format!("{}/{}", kind.dir_name(), key);

        self.facet_usage
            .entry(facet_id.clone())
            .or_default()
            .push(FacetUsageEntry {
                workflow_name: self.workflow_name.to_string(),
                step_name: step_name.to_string(),
                slot: slot.to_string(),
            });

        if !self.all_facet_keys.contains(&facet_id) {
            let item = DiagnosticItem {
                severity: Severity::Error,
                message: format!(
                    "ステップ '{}' が存在しないファセット '{}' ({}) を参照しています",
                    step_name,
                    key,
                    kind.dir_name()
                ),
                workflow_name: Some(self.workflow_name.to_string()),
                step_name: Some(step_name.to_string()),
                facet_key: Some(key.to_string()),
                facet_kind: Some(kind.dir_name().to_string()),
                field: Some(slot.to_string()),
            };
            add_diagnostic(
                self.items,
                self.workflow_summaries,
                self.workflow_name,
                item,
            );
        }
    }

    /// 1 つの step が持つ全 facet ref を一括検査する。
    fn check_step(&mut self, step_name: &str, facet_refs: &FacetRefs<'_>) {
        let singles: &[(&str, FacetKind, Option<&str>)] = &[
            ("policy", FacetKind::Policy, facet_refs.policy),
            ("knowledge", FacetKind::Knowledge, facet_refs.knowledge),
            (
                "instruction",
                FacetKind::Instruction,
                facet_refs.instruction,
            ),
        ];
        for (slot, kind, key_opt) in singles {
            if let Some(key) = key_opt {
                self.check(step_name, slot, *kind, key);
            }
        }
    }
}

fn check_template_variables(
    content: &str,
    facet_key: &str,
    facet_kind_name: &str,
    facet_id: &str,
    items: &mut Vec<DiagnosticItem>,
    facet_summaries: &mut HashMap<String, DiagnosticSummary>,
) {
    for var_name in prompt_rendering::find_undefined_template_variables(content) {
        let item = DiagnosticItem {
            severity: Severity::Error,
            message: format!(
                "ファセット '{}' に未定義のテンプレート変数 '{{{{{}}}}}' が含まれています",
                facet_key, var_name
            ),
            workflow_name: None,
            step_name: None,
            facet_key: Some(facet_key.to_string()),
            facet_kind: Some(facet_kind_name.to_string()),
            field: Some("content".to_string()),
        };
        add_diagnostic(items, facet_summaries, facet_id, item);
    }
}

fn add_diagnostic(
    items: &mut Vec<DiagnosticItem>,
    summaries: &mut HashMap<String, DiagnosticSummary>,
    key: &str,
    item: DiagnosticItem,
) {
    let summary = summaries.entry(key.to_string()).or_default();
    match item.severity {
        Severity::Error => summary.error_count += 1,
        Severity::Warning => summary.warning_count += 1,
        Severity::Info => summary.info_count += 1,
    }
    items.push(item);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CollectConfig, CommandSpec, FacetRefs, FanoutSpec, InterimChild, NodeKind, ReduceStrategy,
        Rule, SchemaDef, SessionSpec, Workflow,
    };
    use std::fs;
    use tempfile::TempDir;

    fn make_step(name: &str, instruction: Option<&str>) -> NodeDefinition {
        let facets = FacetRefs {
            instruction: instruction.map(str::to_string),
            ..Default::default()
        };
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets,
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_child(name: &str, instruction: Option<&str>) -> InterimChild {
        InterimChild {
            name: name.to_string(),
            permission: Some("edit".to_string()),
            facets: FacetRefs {
                instruction: instruction.map(str::to_string),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn make_fanout(name: &str, children: Vec<InterimChild>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                parallel_children: children,
                aggregate: None,
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_command(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet(dir: &Path, kind: &str, key: &str, content: &str) {
        let facet_dir = dir.join(kind);
        fs::create_dir_all(&facet_dir).unwrap();
        fs::write(facet_dir.join(format!("{key}.md")), content).unwrap();
    }

    fn save_workflow_yaml(dir: &Path, wf: &Workflow) {
        fs::create_dir_all(dir).unwrap();
        let content = serde_saphyr::to_string(wf).unwrap();
        fs::write(dir.join(format!("{}.yml", wf.name)), content).unwrap();
    }

    #[test]
    fn diagnose_broken_yaml() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(wf_dir.join("broken.yml"), "invalid: yaml: [[[").unwrap();

        let report = diagnose_all(&wf_dir, &wf_dir);
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error
                    && i.workflow_name.as_deref() == Some("broken"))
        );
    }

    #[test]
    fn diagnose_missing_facet_ref() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("nonexistent-instruction"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("存在しないファセット")));
    }

    #[test]
    fn diagnose_missing_input_schema_ref() {
        // Scenario: input が存在しない schemas Contract キーを参照していれば
        // workflow validation 経由でエラーになる
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                input: Some("nonexistent-contract".to_string()),
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.step_name.as_deref() == Some("step1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_missing_artifact_schema_ref_remains_node_scoped() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                artifact: Some("nonexistent-contract".to_string()),
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.step_name.as_deref() == Some("step1")
                && i.field.as_deref() == Some("artifact")),
            "Expected missing-artifact-schema error on step1, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_array_items_unknown_schema_ref_is_schema_scoped() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "review-list".to_string(),
                SchemaDef::Array {
                    items: "missing-item".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![make_step("step1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("schemas.review-list")
                && i.message
                    .contains("array.items references unknown schemas 'missing-item'")
                && i.step_name.is_none()
                && i.field.as_deref() == Some("schemas")),
            "Expected schema-scoped array.items error, got: {:?}",
            report.items
        );
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.step_name.as_deref() == Some("review-list")),
            "array.items diagnostics must not be attached to a schema name as a step: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_schema_refs_do_not_record_facet_usage() {
        // Scenario: schemas Contract はファセットではないため facet_usage に記録されない
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "input-contract".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                input: Some("input-contract".to_string()),
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.facet_usage.contains_key("contracts/input-contract"),
            "schemas Contract must not be tracked as facet usage: {:?}",
            report.facet_usage
        );
    }

    #[test]
    fn diagnose_missing_input_schema_ref_in_parallel_child() {
        // Scenario: parallel child の input でも存在しない schemas Contract
        // 参照を検出する
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_fanout(
                "parent",
                vec![InterimChild {
                    name: "child1".to_string(),
                    facets: FacetRefs {
                        instruction: Some("impl".to_string()),
                        ..Default::default()
                    },
                    input: Some("nonexistent-contract".to_string()),
                    ..Default::default()
                }],
            )],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.step_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error on child, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_missing_step_ref() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                rules: vec![Rule::Next("nonexistent".to_string())],
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let rule_target_errors = report
            .items
            .iter()
            .filter(|i| {
                i.severity == Severity::Error
                    && i.step_name.as_deref() == Some("step1")
                    && i.field.as_deref() == Some("rules.next")
                    && i.message.contains("存在しないnode")
            })
            .count();
        assert_eq!(
            rule_target_errors, 1,
            "expected one rules target diagnostic from validate_all, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_collect_warning() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    // source step without rules
                    ..make_step("review-step", Some("review"))
                },
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["review-step".to_string()],
                        reduce: ReduceStrategy::AnyNeedsFix,
                    }),
                    ..make_step("collect-step", None)
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message.contains("rulesが未設定")));
    }

    #[test]
    fn diagnose_unreachable_step() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // step1 が Auto + rules で step3 へ遷移 → orphan は到達不能
        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    rules: vec![Rule::Next("step3".to_string())],
                    ..make_step("step1", Some("impl"))
                },
                make_step("orphan", Some("impl")),
                make_step("step3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Warning
                && i.message.contains("到達不能")
                && i.step_name.as_deref() == Some("orphan")),
            "Expected unreachable warning for orphan, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_rules_without_fallthrough_marks_later_nodes_unreachable() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // rules なしの node は終端なので、定義順の暗黙到達はない。
        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_step("step1", Some("impl")),
                make_step("step2", Some("impl")),
                make_step("step3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for step_name in ["step2", "step3"] {
            assert!(
                report.items.iter().any(|i| i.severity == Severity::Warning
                    && i.message.contains("到達不能")
                    && i.step_name.as_deref() == Some(step_name)),
                "Expected unreachable warning for {step_name}, got: {:?}",
                report.items
            );
        }
    }

    #[test]
    fn diagnose_builtin_workflow_info() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインワークフロー")));
    }

    #[test]
    fn diagnose_builtin_facet_info() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインファセット")));
    }

    #[test]
    fn diagnose_template_variable_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "bad", "Use {{request.field}} here");
        let wf = Workflow {
            name: "bad-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("bad")
            && i.message
                .contains("未定義のテンプレート変数 '{{request.field}}'")));
    }

    #[test]
    fn diagnose_request_reference_ok() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "good", "Request: {{ request }}");

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(!report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("good")
            && i.message.contains("未定義のテンプレート変数")));
    }

    /// command node は command を持ち facet は不要。
    /// diagnose_all 経路で valid な command node が誤って「ファセット参照が必要」
    /// エラーにならないことを担保する（validation.rs と同じ整合性が diagnostics 側にも必要）。
    #[test]
    fn diagnose_command_node_with_command_has_no_facet_required_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = Workflow {
            name: "bash-wf".to_string(),
            description: "bash test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_command("build", "cargo build")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("build")
                && i.message.contains("ファセット参照")),
            "command node with command must not trigger facet requirement error: {:?}",
            report.items
        );
    }

    /// command node の command が空なら validation 経路で command field のエラーになる。
    #[test]
    fn diagnose_command_node_with_empty_command_reports_command_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = Workflow {
            name: "bash-wf".to_string(),
            description: "bash test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_command("build", "   ")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.message.contains("command")),
            "bash node without command must report a command-related error: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_facet_usage_tracked() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let usage = report.facet_usage.get("instructions/impl");
        assert!(usage.is_some());
        assert_eq!(usage.unwrap().len(), 1);
        assert_eq!(usage.unwrap()[0].workflow_name, "test-wf");
    }

    #[test]
    fn diagnose_workflow_name_invalid() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // ファイル名が不正な文字を含むworkflowを作成
        // load_workflow内のvalidation::validateで名前が拒否されるため、
        // diagnose_allでは「読み込みに失敗」エラーとして報告される
        fs::create_dir_all(wf_dir).unwrap();
        let wf = Workflow {
            name: "bad workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("impl"))],
        };
        let content = serde_saphyr::to_string(&wf).unwrap();
        fs::write(wf_dir.join("bad workflow.yml"), content).unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("命名規則に違反")));
    }

    #[test]
    fn diagnose_collect_warning_all_passed() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    ..make_step("review-step", Some("review"))
                },
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["review-step".to_string()],
                        reduce: ReduceStrategy::AllPassed,
                    }),
                    ..make_step("collect-step", None)
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message.contains("rulesが未設定")));
    }

    #[test]
    fn diagnose_invalid_facet_key_via_diagnose_all() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // 不正な文字を含むファセットキーファイルを直接作成
        let policies_dir = wf_dir.join("policies");
        fs::create_dir_all(&policies_dir).unwrap();
        fs::write(policies_dir.join("bad key!.md"), "content").unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("命名規則")
            && i.facet_key.as_deref() == Some("bad key!")));
    }

    #[test]
    fn diagnose_invalid_schema_identifier_via_diagnose_all() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");
        fs::create_dir_all(wf_dir).unwrap();
        fs::write(
            wf_dir.join("bad-schema-name.yml"),
            r#"name: bad-schema-name
description: test
schemas:
  "review; curl https://example.invalid #":
    type: object
    properties:
      status: string
    required:
      - status
nodes:
  - name: review
    session:
      permission: edit
      facets:
        instruction: review
    artifact: "review; curl https://example.invalid #"
"#,
        )
        .unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.workflow_name.as_deref() == Some("bad-schema-name")
            && i.field.as_deref() == Some("schemas")
            && i.message.contains("must start with an ASCII alphanumeric")));
    }

    #[test]
    fn diagnose_schema_violation_yaml() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        fs::create_dir_all(wf_dir).unwrap();
        // Valid YAML but missing required `steps` field
        fs::write(
            wf_dir.join("bad-schema.yml"),
            "name: bad-schema\ndescription: test\n",
        )
        .unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("bad-schema")),
            "Expected error for schema-violating workflow, got: {:?}",
            report.items
        );
    }

    // [02]: 新 schema では kind block が型レベルで必須となるため、旧テスト
    // `diagnose_missing_mode_via_validation` は YAML deserialize 段階で吸収されるため削除した。

    #[test]
    fn diagnose_duplicate_step_via_validation() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "dup-step".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_step("same-name", Some("task")),
                make_step("same-name", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("dup-step")
                && i.message.contains("重複")),
            "Expected duplicate step error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_node_input_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "input-ref".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "artifact".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                NodeDefinition {
                    artifact: Some("artifact".to_string()),
                    ..make_step("step1", Some("task"))
                },
                NodeDefinition {
                    inputs: vec!["step1".to_string()],
                    artifact: Some("artifact".to_string()),
                    ..make_step("step2", Some("task"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.field.as_deref() == Some("inputs")),
            "Artifact input reference should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_collect_from_subsequent_step() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        // step1 が後続の step2 を collect.from で参照 → エラーになるべき
        let wf = Workflow {
            name: "subsequent-collect".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["step2".to_string()],
                        reduce: ReduceStrategy::Concat,
                    }),
                    ..make_step("step1", None)
                },
                make_step("step2", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("step1")
                && i.field.as_deref() == Some("collect.from")
                && i.message.contains("まだ定義されていないステップ")),
            "Expected subsequent step reference error for collect.from, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_parallel_child_item_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "{{ item.path }}");

        let wf = Workflow {
            name: "par-item".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_fanout(
                "par",
                vec![InterimChild {
                    name: "child1".to_string(),
                    facets: FacetRefs {
                        instruction: Some("task".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
            )],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("inputs")),
            "item reference inside parallel child should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_fanout_inputs_rejected() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let mut fanout = make_fanout(
            "par",
            vec![
                make_child("child1", Some("task")),
                make_child("child2", Some("task")),
            ],
        );
        fanout.inputs = vec!["request".to_string()];
        let wf = Workflow {
            name: "fanout-inputs".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![fanout],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("fanout-inputs")
                && i.field.as_deref() == Some("inputs")
                && i.message.contains("fanout")),
            "Expected fanout inputs error, got: {:?}",
            report.items
        );
    }
}
