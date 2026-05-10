use super::builtin;
use super::facet::{self, FacetKind};
use super::schema::{ReduceStrategy, Step, StepMode, Workflow};
use super::validation;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const ALL_FACET_KINDS: [FacetKind; 5] = [
    FacetKind::Persona,
    FacetKind::Policy,
    FacetKind::Knowledge,
    FacetKind::Instruction,
    FacetKind::OutputContract,
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
    let workflows = load_all_workflows(workflows_dir);
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
fn load_all_workflows(dir: &Path) -> Vec<(String, Result<Workflow, String>)> {
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
                                serde_saphyr::from_str::<Workflow>(&content)
                                    .map_err(|e| e.to_string())
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
            match builtin::get_builtin_workflow(&summary.name) {
                Some(wf) => results.push((summary.name, Ok(wf))),
                None => results.push((
                    summary.name.clone(),
                    Err(format!(
                        "ビルトインワークフロー '{}' の読み込みに失敗",
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
            | ValidationError::UnknownNextStep { .. }
            | ValidationError::MissingFacet { .. }
            | ValidationError::UnknownOutputFrom { .. }
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
        ValidationError::MissingMode { step } => (Some(step.clone()), Some("mode".to_string())),
        ValidationError::ParallelBlockHasMode { step } => {
            (Some(step.clone()), Some("mode".to_string()))
        }
        ValidationError::ParallelChildNotAuto { parent, .. } => {
            (Some(parent.clone()), Some("parallel.mode".to_string()))
        }
        ValidationError::ParallelChildNameConflict { child } => {
            (Some(child.clone()), Some("parallel.name".to_string()))
        }
        ValidationError::AggregateWithoutParallel { step } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::AggregateInvalidConfig { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::AggregateUnknownTarget { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::ParallelChildSiblingRef { parent, .. } => (
            Some(parent.clone()),
            Some("parallel.pass_output_from".to_string()),
        ),
        ValidationError::ParallelChildMissingFacet { parent, .. } => {
            (Some(parent.clone()), Some("parallel.facets".to_string()))
        }
        ValidationError::UnknownNextStep { step, .. } => {
            (Some(step.clone()), Some("rules.next".to_string()))
        }
        ValidationError::MissingFacet { step } => (Some(step.clone()), Some("facets".to_string())),
        ValidationError::UnknownOutputFrom { step, .. } => {
            (Some(step.clone()), Some("pass_output_from".to_string()))
        }
        ValidationError::UnknownCollectFrom { step, .. } => {
            (Some(step.clone()), Some("collect.from".to_string()))
        }
        ValidationError::UnknownOnExhausted { step, .. } => {
            (Some(step.clone()), Some("cycle_guard.on_exhausted".to_string()))
        }
        ValidationError::UnknownResetsCycleFor { step, .. } => {
            (Some(step.clone()), Some("resets_cycle_for".to_string()))
        }
        ValidationError::CircularOnExhausted { cycle } => {
            (cycle.first().cloned(), Some("cycle_guard.on_exhausted".to_string()))
        }
        ValidationError::ResetsCycleForNonGuardedStep { step, .. } => {
            (Some(step.clone()), Some("resets_cycle_for".to_string()))
        }
    }
}

/// ステップが定義順で次のステップへ暗黙的に進行し得るかを判定。
/// エンジンの実際の遷移ロジックに基づく:
/// - rules なし → 全モードで定義順の次へ進行
/// - Interactive モード → rules マッチなし時に進行
/// - Approval モード → approve 時に進行
/// - Parallel block (aggregate なし) → 子完了後に進行
/// - Auto モード + rules あり → マッチで遷移 or 不一致で FAIL（進行しない）
fn can_advance_sequentially(step: &Step) -> bool {
    if step.is_parallel_block() {
        return step.aggregate.is_none();
    }
    if step.rules.is_empty() {
        return true;
    }
    matches!(
        step.mode,
        Some(StepMode::Interactive) | Some(StepMode::Approval)
    )
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

    if let Some(first) = wf.steps.first() {
        explicitly_reachable.insert(&first.name);
    }

    // 明示的遷移先を収集
    for step in &wf.steps {
        for rule in &step.rules {
            if step_names.contains(rule.next.as_str()) {
                explicitly_reachable.insert(&rule.next);
            }
        }
        if let Some(ref agg) = step.aggregate {
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
        for (i, step) in wf.steps.iter().enumerate() {
            if !all_reachable.contains(step.name.as_str()) {
                continue;
            }
            if i + 1 >= wf.steps.len() {
                continue;
            }
            let next = &wf.steps[i + 1];
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
    for e in validation::validate_all(wf) {
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
    let step_names: HashSet<&str> = wf.steps.iter().map(|s| s.name.as_str()).collect();
    // 参照可能名前空間: トップレベルstep名 + 並列子step名（pass_output_from等の検証用）
    // validation.rs の referenceable_step_names と同じロジック
    let mut referenceable_step_names: HashSet<&str> = HashSet::new();
    for step in &wf.steps {
        referenceable_step_names.insert(step.name.as_str());
        if let Some(ref children) = step.parallel {
            for child in children {
                referenceable_step_names.insert(child.name.as_str());
            }
        }
    }
    // 先行step名の集合（collect.from のチェック用）
    // validation.rs と同じロジック: collect.from は先行ステップのみ参照可能
    let mut preceding_step_names: HashSet<&str> = HashSet::new();
    let reachability = compute_reachable_steps(wf, &step_names);

    // 各stepを診断
    for step in &wf.steps {
        // step参照チェック（rules.next）
        for rule in &step.rules {
            if !step_names.contains(rule.next.as_str()) {
                let item = DiagnosticItem {
                    severity: Severity::Error,
                    message: format!(
                        "ステップ '{}' のルールが存在しないステップ '{}' を参照しています",
                        step.name, rule.next
                    ),
                    workflow_name: Some(name.clone()),
                    step_name: Some(step.name.clone()),
                    facet_key: None,
                    facet_kind: None,
                    field: Some("rules.next".to_string()),
                };
                add_diagnostic(items, workflow_summaries, name, item);
            }
        }

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
                    if let Some(source_step) = wf.steps.iter().find(|s| s.name == *from) {
                        if source_step.rules.is_empty() && !source_step.is_parallel_block() {
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

        // pass_output_from 参照チェック（後方参照も許可、validation.rsと同じ）
        if let Some(ref refs) = step.pass_output_from {
            for r in refs {
                if !referenceable_step_names.contains(r.as_str()) {
                    let item = DiagnosticItem {
                        severity: Severity::Error,
                        message: format!(
                            "ステップ '{}' のpass_output_fromが存在しないステップ '{}' を参照しています",
                            step.name, r
                        ),
                        workflow_name: Some(name.clone()),
                        step_name: Some(step.name.clone()),
                        facet_key: None,
                        facet_kind: None,
                        field: Some("pass_output_from".to_string()),
                    };
                    add_diagnostic(items, workflow_summaries, name, item);
                }
            }
        }

        // ファセット参照の存在チェック + usage 記録
        check_step_facet_refs(
            &step.name,
            &FacetRefs {
                persona: step.persona.as_deref(),
                policy: step.policy.as_deref(),
                knowledge: step.knowledge.as_deref(),
                instruction: step.instruction.as_deref(),
                output_contract: step.output_contract.as_deref(),
            },
            name,
            all_facet_keys,
            items,
            workflow_summaries,
            facet_usage,
        );

        // ファセット未設定チェック（inline_prompt があればOK）
        if !step.is_parallel_block()
            && step.collect.is_none()
            && !step.has_facet_refs()
            && step.inline_prompt.is_none()
        {
            let item = DiagnosticItem {
                severity: Severity::Error,
                message: format!(
                    "ステップ '{}' にはファセット参照またはinline_promptが必要です",
                    step.name
                ),
                workflow_name: Some(name.clone()),
                step_name: Some(step.name.clone()),
                facet_key: None,
                facet_kind: None,
                field: Some("facets".to_string()),
            };
            add_diagnostic(items, workflow_summaries, name, item);
        }

        // parallel block の子step 診断
        if let Some(ref children) = step.parallel {
            let child_names: HashSet<&str> = children.iter().map(|c| c.name.as_str()).collect();
            for child in children {
                check_step_facet_refs(
                    &child.name,
                    &FacetRefs {
                        persona: child.persona.as_deref(),
                        policy: child.policy.as_deref(),
                        knowledge: child.knowledge.as_deref(),
                        instruction: child.instruction.as_deref(),
                        output_contract: child.output_contract.as_deref(),
                    },
                    name,
                    all_facet_keys,
                    items,
                    workflow_summaries,
                    facet_usage,
                );

                // 並列子stepの pass_output_from チェック（兄弟参照禁止、後方参照は許可）
                if let Some(ref refs) = child.pass_output_from {
                    for r in refs {
                        if child_names.contains(r.as_str()) {
                            let item = DiagnosticItem {
                                severity: Severity::Error,
                                message: format!(
                                    "parallelブロック '{}' の子ステップ '{}' のpass_output_fromが同一ブロック内の兄弟ステップ '{}' を参照しています",
                                    step.name, child.name, r
                                ),
                                workflow_name: Some(name.clone()),
                                step_name: Some(child.name.clone()),
                                facet_key: None,
                                facet_kind: None,
                                field: Some("parallel.pass_output_from".to_string()),
                            };
                            add_diagnostic(items, workflow_summaries, name, item);
                        } else if !referenceable_step_names.contains(r.as_str()) {
                            let item = DiagnosticItem {
                                severity: Severity::Error,
                                message: format!(
                                    "parallelブロック '{}' の子ステップ '{}' のpass_output_fromが存在しないステップ '{}' を参照しています",
                                    step.name, child.name, r
                                ),
                                workflow_name: Some(name.clone()),
                                step_name: Some(child.name.clone()),
                                facet_key: None,
                                facet_kind: None,
                                field: Some("parallel.pass_output_from".to_string()),
                            };
                            add_diagnostic(items, workflow_summaries, name, item);
                        }
                    }
                }
            }
        }

        // 到達可能性チェック（最初のstepを除く）
        if wf.steps.first().map(|s| &s.name) != Some(&step.name) {
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
        if let Some(ref children) = step.parallel {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }
}

struct FacetRefs<'a> {
    persona: Option<&'a str>,
    policy: Option<&'a str>,
    knowledge: Option<&'a str>,
    instruction: Option<&'a str>,
    output_contract: Option<&'a str>,
}

fn check_step_facet_refs(
    step_name: &str,
    facet_refs: &FacetRefs<'_>,
    workflow_name: &str,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
    let refs: Vec<(&str, FacetKind, Option<&str>)> = vec![
        ("persona", FacetKind::Persona, facet_refs.persona),
        ("policy", FacetKind::Policy, facet_refs.policy),
        ("knowledge", FacetKind::Knowledge, facet_refs.knowledge),
        (
            "instruction",
            FacetKind::Instruction,
            facet_refs.instruction,
        ),
        (
            "output_contract",
            FacetKind::OutputContract,
            facet_refs.output_contract,
        ),
    ];

    for (slot, kind, key_opt) in refs {
        if let Some(key) = key_opt {
            let facet_id = format!("{}/{}", kind.dir_name(), key);

            // usage 記録
            facet_usage
                .entry(facet_id.clone())
                .or_default()
                .push(FacetUsageEntry {
                    workflow_name: workflow_name.to_string(),
                    step_name: step_name.to_string(),
                    slot: slot.to_string(),
                });

            // 存在チェック
            if !all_facet_keys.contains(&facet_id) {
                let item = DiagnosticItem {
                    severity: Severity::Error,
                    message: format!(
                        "ステップ '{}' が存在しないファセット '{}' ({}) を参照しています",
                        step_name,
                        key,
                        kind.dir_name()
                    ),
                    workflow_name: Some(workflow_name.to_string()),
                    step_name: Some(step_name.to_string()),
                    facet_key: Some(key.to_string()),
                    facet_kind: Some(kind.dir_name().to_string()),
                    field: Some(slot.to_string()),
                };
                add_diagnostic(items, workflow_summaries, workflow_name, item);
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
    for var_name in facet::find_undefined_template_variables(content) {
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
    use crate::workflow::schema::{
        CollectConfig, ReduceStrategy, Step, StepMode, TransitionRule, Workflow,
    };
    use std::fs;
    use tempfile::TempDir;

    fn make_step(name: &str, instruction: Option<&str>) -> Step {
        Step {
            name: name.to_string(),
            mode: Some(StepMode::Auto),
            persona: None,
            policy: None,
            knowledge: None,
            instruction: instruction.map(|s| s.to_string()),
            output_contract: None,
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
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
            steps: vec![make_step("step1", Some("nonexistent-instruction"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("存在しないファセット")));
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
            steps: vec![Step {
                rules: vec![TransitionRule {
                    r#match: "DONE".to_string(),
                    next: "nonexistent".to_string(),
                }],
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("存在しないステップ")));
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
            steps: vec![
                Step {
                    // source step without rules
                    ..make_step("review-step", Some("review"))
                },
                Step {
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
            steps: vec![
                Step {
                    rules: vec![TransitionRule {
                        r#match: "NEXT".to_string(),
                        next: "step3".to_string(),
                    }],
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
    fn diagnose_sequential_fallthrough_not_unreachable_but_warns() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // rules なしの step → 次の step は暗黙的に到達可能（到達不能ではない）
        // ただし明示的遷移なしの warning は出る
        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                make_step("step1", Some("impl")),
                make_step("step2", Some("impl")),
                make_step("step3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        // 到達不能ではない
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.severity == Severity::Warning && i.message.contains("到達不能")),
            "Sequential steps should not be flagged as unreachable, got: {:?}",
            report.items
        );
        // 明示的遷移なしの warning が出る
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Warning
                && i.message.contains("明示的な遷移が定義されていません")
                && i.step_name.as_deref() == Some("step2")),
            "Expected implicit-only warning for step2, got: {:?}",
            report.items
        );
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
        setup_facet(wf_dir, "instructions", "bad", "Use {{unknown_var}} here");

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(
            |i| i.severity == Severity::Error && i.message.contains("未定義のテンプレート変数")
        ));
    }

    #[test]
    fn diagnose_system_variables_ok() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(
            wf_dir,
            "instructions",
            "good",
            "Project: {{project_name}}, Task: {{task}}",
        );

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(!report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("good")
            && i.message.contains("未定義のテンプレート変数")));
    }

    #[test]
    fn diagnose_inline_prompt_no_facet_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![Step {
                inline_prompt: Some("Do analysis".to_string()),
                ..make_step("step1", None)
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(!report.items.iter().any(|i| i.severity == Severity::Error
            && i.step_name.as_deref() == Some("step1")
            && i.message.contains("ファセット参照")));
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
            steps: vec![make_step("step1", Some("impl"))],
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
            steps: vec![make_step("step1", Some("impl"))],
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
            steps: vec![
                Step {
                    ..make_step("review-step", Some("review"))
                },
                Step {
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

    #[test]
    fn diagnose_missing_mode_via_validation() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "no-mode".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![Step {
                mode: None,
                instruction: Some("task".to_string()),
                ..make_step("step-1", None)
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("no-mode")
                && i.message.contains("mode")),
            "Expected mode-missing error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_duplicate_step_via_validation() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "dup-step".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
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
    fn diagnose_aggregate_without_parallel_via_validation() {
        use crate::workflow::schema::AggregateConfig;

        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "agg-no-par".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    aggregate: Some(AggregateConfig {
                        all_match: Some("pass".to_string()),
                        any_match: None,
                        then: "step-2".to_string(),
                        r#else: "step-2".to_string(),
                    }),
                    ..make_step("step-1", Some("task"))
                },
                make_step("step-2", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("agg-no-par")
                && i.message.contains("aggregate")),
            "Expected aggregate-without-parallel error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_pass_output_from_backward_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        // step1 が後続の step2 を pass_output_from で参照 → 後方参照は許可（エラーにならない）
        let wf = Workflow {
            name: "backward-ref".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    pass_output_from: Some(vec!["step2".to_string()]),
                    ..make_step("step1", Some("task"))
                },
                make_step("step2", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.field.as_deref() == Some("pass_output_from")),
            "Backward reference in pass_output_from should not be an error, got: {:?}",
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
            steps: vec![
                Step {
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
    fn diagnose_parallel_child_backward_reference_passes() {
        use crate::workflow::schema::ParallelStep;

        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");
        setup_facet(wf_dir, "personas", "reviewer", "content");

        // parallel block の子step が後続の report を参照 → 後方参照は許可（エラーにならない）
        let wf = Workflow {
            name: "par-backward".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    parallel: Some(vec![ParallelStep {
                        name: "child1".to_string(),
                        mode: StepMode::Auto,
                        persona: Some("reviewer".to_string()),
                        policy: None,
                        knowledge: None,
                        instruction: Some("task".to_string()),
                        output_contract: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["report".to_string()]),
                    }]),
                    ..make_step("par", None)
                },
                make_step("report", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("parallel.pass_output_from")),
            "Backward reference in parallel child pass_output_from should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_parallel_child_sibling_ref() {
        use crate::workflow::schema::ParallelStep;

        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");
        setup_facet(wf_dir, "personas", "reviewer", "content");

        // parallel block の子step が兄弟を参照 → エラーになるべき
        let wf = Workflow {
            name: "par-sibling".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![Step {
                parallel: Some(vec![
                    ParallelStep {
                        name: "child1".to_string(),
                        mode: StepMode::Auto,
                        persona: Some("reviewer".to_string()),
                        policy: None,
                        knowledge: None,
                        instruction: Some("task".to_string()),
                        output_contract: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                    },
                    ParallelStep {
                        name: "child2".to_string(),
                        mode: StepMode::Auto,
                        persona: Some("reviewer".to_string()),
                        policy: None,
                        knowledge: None,
                        instruction: Some("task".to_string()),
                        output_contract: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["child1".to_string()]),
                    },
                ]),
                ..make_step("par", None)
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("child2")
                && i.field.as_deref() == Some("parallel.pass_output_from")
                && i.message.contains("兄弟ステップ")),
            "Expected sibling reference error for parallel child, got: {:?}",
            report.items
        );
    }
}
