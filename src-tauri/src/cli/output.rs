use std::path::{Path, PathBuf};

use clap::Subcommand;

use super::common::{validate_run_id, CliError};
use super::workflow_io;
use crate::domain::workflow::{contract, secret_masker, ContractValidationResult};
use crate::usecase::workflow::event_draft;
use crate::usecase::workflow::ports::WorkflowEventDraft;

#[derive(Subcommand, Debug)]
pub(super) enum OutputSubcommand {
    /// node の `artifact` schema に従う構造化出力を提出する。
    /// `--json` と `--file` は相互排他であり、いずれか必須。
    Submit {
        run_id: String,
        #[arg(long = "node", alias = "step", value_name = "NODE_NAME")]
        step: String,
        #[arg(long = "type", value_name = "CONTRACT")]
        contract: String,
        #[arg(long, conflicts_with = "file", value_name = "JSON")]
        json: Option<String>,
        #[arg(long, conflicts_with = "json", value_name = "PATH")]
        file: Option<PathBuf>,
    },
    /// 構造化出力の `artifact` schema 適合性を副作用なしで確認する。
    /// engine state / event log は変化しない。
    Validate {
        run_id: String,
        #[arg(long = "node", alias = "step", value_name = "NODE_NAME")]
        step: String,
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },
    /// 提出済みの構造化出力を取得する。未提出時は決定論的に「未提出」を返す。
    Get {
        run_id: String,
        #[arg(long = "node", alias = "step", value_name = "NODE_NAME")]
        step: String,
        #[arg(long)]
        json: bool,
    },
}

/// [08] `releash workflow output submit`: 構造化出力を pending command として書き出す。
/// engine 到達は稼働中アプリの watcher が担う（spec [08] CLI 完了基準境界）。
///
/// CLI 入口で同期的に以下を検証し、不適合な入力は pending file を作らずに
/// `CliError::InvalidInput` として返す（spec [08]:「不適合な入力は決定論的な
/// validation error として CLI 終了コードで返り、`step_outputs` / 事実履歴は
/// 副作用なしの状態に保たれる」）。
///
/// 検証項目:
///   - run が存在する（event log の RunStarted から workflow を解決）
///   - step が workflow に存在し artifact を持つ
///   - caller の `--type` が step の expected contract と一致する
///   - 入力 JSON が contract 適合（pure validator 再利用）
///
/// stale step 判定（受付中であるか）は engine の権威に委ねる。
pub(super) fn cmd_output_submit(
    data_dir: &Path,
    run_id: &str,
    step: &str,
    contract: &str,
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<String, CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    validate_contract_argument(contract)?;
    let raw_json = read_submit_input_json(json_arg, file_arg)?;
    let structured_output: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| CliError::InvalidInput(format!("Failed to parse JSON: {e}")))?;

    // 同期検証: run / step / contract type 一致 / contract validation。
    let context = resolve_node_artifact_schema_via_log(data_dir, run_id, step)?;
    if context.contract != contract {
        return Err(CliError::InvalidInput(format!(
            "contract mismatch: step '{step}' expects '{}', got '{contract}'",
            context.contract
        )));
    }
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、domain の secret masking + schemas validation 経由で呼ぶ。
    // CLI は別プロセスでアプリ状態 (`AppConfig` / `AppHandle`) を持たないため、ここでは
    // `secrets = &[]` で呼び、最終的な masking 込み判定は engine 側 watcher 経由で再評価される
    // （spec [08] CLI 完了基準: pending を書き出した時点で CLI は完了、最終判定は engine 側）。
    match validate_cli_artifact_output(&context, structured_output.clone()) {
        ContractValidationResult::Valid { .. } => {}
        ContractValidationResult::Invalid(violation) => {
            return Err(CliError::InvalidInput(format!(
                "artifact schema violation ({}): {}",
                violation.reason, violation.details
            )));
        }
    }

    let output = workflow_io::enqueue_pending_command(
        data_dir,
        run_id,
        workflow_io::CliRequestPayload::SubmitOutput {
            step_name: step.to_string(),
            contract: contract.to_string(),
            structured_output,
        },
    )?;
    Ok(format!("{}\n", output.format_stdout_line()))
}

/// [08] `releash workflow output validate`: contract 適合性のみを確認する。
/// pending file / event log / state のいずれにも触れない（spec [08] 振る舞い定義 Rule 2）。
pub(super) fn cmd_output_validate(
    data_dir: &Path,
    run_id: &str,
    step: &str,
    file: &Path,
) -> Result<String, CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    let context = resolve_node_artifact_schema_via_log(data_dir, run_id, step)?;
    let raw_json = std::fs::read_to_string(file)
        .map_err(|e| CliError::InvalidInput(format!("Failed to read file {:?}: {e}", file)))?;
    let value: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| CliError::InvalidInput(format!("Failed to parse JSON: {e}")))?;
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、domain の secret masking + schemas validation 経由で呼ぶ。
    // CLI は別プロセスでアプリ状態を持たないため、ここでは `secrets = &[]` で呼ぶ
    // （最終 masking 込み judging は engine 側で再評価される）。
    match validate_cli_artifact_output(&context, value) {
        ContractValidationResult::Valid { .. } => Ok(format!(
            "ok: artifact schema '{}' is satisfied\n",
            context.contract
        )),
        ContractValidationResult::Invalid(violation) => Err(CliError::InvalidInput(format!(
            "artifact schema violation ({}): {}",
            violation.reason, violation.details
        ))),
    }
}

/// [08] `releash workflow output get`: 提出済みの構造化出力と付随メタを取得する。
/// 未提出の場合は決定論的に「未提出」を返す（spec [08] 振る舞い定義 Rule 3）。
pub(super) fn cmd_output_get(
    data_dir: &Path,
    run_id: &str,
    step: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    workflow_io::ensure_run_exists(data_dir, run_id)?;
    // [08] 振る舞い定義 Rule 3 (5-5 修正): step が workflow に存在しない場合は
    // `output validate` と対称に `InvalidInput` を返す。`not_submitted` 出力は
    // 「step は存在するが未提出」専用とする。
    let _contract = resolve_node_artifact_contract_via_log(data_dir, run_id, step)?;
    let events = workflow_io::read_domain_log(data_dir, run_id)?;
    let view = build_output_get_view(events, step);
    if json {
        let text =
            serde_json::to_string_pretty(&view).map_err(|e| format!("serialize output: {e}"))?;
        Ok(format!("{text}\n"))
    } else {
        match &view {
            OutputGetView::Submitted {
                contract,
                structured_output,
                submitted_at,
                request_id,
                timestamp,
            } => {
                let mut output = format!("submitted: step={step} contract={contract}\n");
                if let Some(sa) = submitted_at {
                    output.push_str(&format!("submitted_at: {sa}\n"));
                }
                if let Some(rid) = request_id {
                    output.push_str(&format!("request_id: {rid}\n"));
                }
                output.push_str(&format!("timestamp: {timestamp}\n"));
                output.push_str(&format!(
                    "structured_output:\n{}\n",
                    serde_json::to_string_pretty(structured_output)
                        .map_err(|e| format!("serialize structured_output: {e}"))?
                ));
                Ok(output)
            }
            OutputGetView::NotSubmitted => Ok(format!("not_submitted: step={step}\n")),
        }
    }
}

fn validate_step_argument(step: &str) -> Result<(), CliError> {
    if step.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--node must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_contract_argument(contract: &str) -> Result<(), CliError> {
    if contract.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--type must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn read_submit_input_json(
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<String, CliError> {
    match (json_arg, file_arg) {
        (Some(_), Some(_)) => Err(CliError::InvalidInput(
            "--json and --file are mutually exclusive".to_string(),
        )),
        (Some(raw), None) => Ok(raw),
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map_err(|e| CliError::InvalidInput(format!("Failed to read file {:?}: {e}", path))),
        (None, None) => Err(CliError::InvalidInput(
            "either --json or --file is required".to_string(),
        )),
    }
}

/// event log の `RunStarted` から workflow definition を取り出し、node の
/// `artifact` を解決する。
///
/// 経路本体は usecase の event draft helper に委譲し、CLI 層は
/// `ContractLookupError` を `CliError` に射影するだけを担う。
fn resolve_node_artifact_contract_via_log(
    data_dir: &Path,
    run_id: &str,
    step: &str,
) -> Result<String, CliError> {
    let events = workflow_io::read_domain_log(data_dir, run_id)?;
    event_draft::resolve_node_artifact_contract_from_drafts(&events, step, run_id).map_err(|err| {
        match err {
            contract::ContractLookupError::RunNotFound { .. } => {
                CliError::NotFound(format!("Workflow run not found: {run_id}"))
            }
            contract::ContractLookupError::InvalidRunStartedPayload { details } => {
                CliError::InvalidInput(details)
            }
            contract::ContractLookupError::NoArtifactContract {
                workflow_name,
                node,
            } => CliError::InvalidInput(format!(
                "node '{node}' has no artifact in workflow '{workflow_name}'"
            )),
        }
    })
}

fn resolve_node_artifact_schema_via_log(
    data_dir: &Path,
    run_id: &str,
    step: &str,
) -> Result<event_draft::ArtifactSchemaContext, CliError> {
    let events = workflow_io::read_domain_log(data_dir, run_id)?;
    event_draft::resolve_node_artifact_schema_from_drafts(&events, step, run_id).map_err(|err| {
        match err {
            contract::ContractLookupError::RunNotFound { .. } => {
                CliError::NotFound(format!("Workflow run not found: {run_id}"))
            }
            contract::ContractLookupError::InvalidRunStartedPayload { details } => {
                CliError::InvalidInput(details)
            }
            contract::ContractLookupError::NoArtifactContract {
                workflow_name,
                node,
            } => CliError::InvalidInput(format!(
                "node '{node}' has no artifact in workflow '{workflow_name}'"
            )),
        }
    })
}

fn validate_cli_artifact_output(
    context: &event_draft::ArtifactSchemaContext,
    structured_output: serde_json::Value,
) -> ContractValidationResult {
    let redacted =
        secret_masker::mask_sensitive_structured_output(&context.contract, structured_output, &[]);
    contract::validate_artifact_value(&context.schemas, &context.contract, redacted)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum OutputGetView {
    Submitted {
        contract: String,
        structured_output: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

fn build_output_get_view(events: Vec<WorkflowEventDraft>, step: &str) -> OutputGetView {
    // 最新の ArtifactProduced は pure projection helper（spec [08] L165 / 振る舞い定義
    // Rule 3）に集約する。CLI / Tauri 経路はそれぞれ自層の DTO（OutputGetView /
    // WorkflowGetOutputResponse）へ map するだけで挙動を共有する。
    match event_draft::latest_artifact_produced_from_drafts(&events, step) {
        Some(snapshot) => OutputGetView::Submitted {
            contract: snapshot.contract,
            structured_output: snapshot.value,
            submitted_at: snapshot.submitted_at,
            request_id: snapshot.request_id,
            timestamp: snapshot.timestamp,
        },
        None => OutputGetView::NotSubmitted,
    }
}

#[cfg(test)]
mod tests {
    use super::super::common::test_support::*;
    use super::super::workflow_io::read_domain_log;
    use super::super::Cli;
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
    use crate::adaptor::gateway::workflow::pending_command::PendingCommandStore;
    use crate::adaptor::gateway::workflow::run::RunStatus;
    use crate::adaptor::gateway::workflow::schema::SchemaDef;
    use clap::Parser;
    use tempfile::TempDir;

    fn object_schema(fields: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: fields
                .iter()
                .map(|field| (field.to_string(), SchemaDef::String { r#enum: None }))
                .collect(),
            required: fields.iter().map(|field| field.to_string()).collect(),
            additional_properties: false,
        }
    }

    fn test_schemas() -> std::collections::BTreeMap<String, SchemaDef> {
        [
            ("review-verdict".to_string(), object_schema(&["verdict"])),
            ("spec-directory".to_string(), object_schema(&["spec_dir"])),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn cli_workflow_output_subcommands_parse_via_clap() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec![
                "releash",
                "workflow",
                "output",
                "submit",
                run_id,
                "--step",
                "review",
                "--type",
                "review-verdict",
                "--json",
                "{\"verdict\":\"LGTM\"}",
            ],
            vec![
                "releash",
                "workflow",
                "output",
                "submit",
                run_id,
                "--step",
                "review",
                "--type",
                "review-verdict",
                "--file",
                "out.json",
            ],
            vec![
                "releash", "workflow", "output", "validate", run_id, "--step", "review", "--file",
                "out.json",
            ],
            vec![
                "releash", "workflow", "output", "get", run_id, "--step", "review",
            ],
            vec![
                "releash", "workflow", "output", "get", run_id, "--step", "review", "--json",
            ],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "parser must accept workflow output subcommand: {argv:?}"
            );
        }
    }

    /// [08] CLI 入力境界: `submit` の `--json` と `--file` は相互排他。両方指定された
    /// 場合は parser が reject する。
    #[test]
    fn cli_workflow_output_submit_rejects_both_json_and_file() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        let argv = vec![
            "releash",
            "workflow",
            "output",
            "submit",
            run_id,
            "--step",
            "review",
            "--type",
            "review-verdict",
            "--json",
            "{}",
            "--file",
            "out.json",
        ];
        assert!(Cli::try_parse_from(&argv).is_err());
    }

    /// テスト用 helper: 指定 step に artifact を持つ workflow を含む RunStarted
    /// event を log に append し、run_file も書き込む。CLI submit / validate の synchronous
    /// 解決経路は event log の RunStarted から workflow を取り出すため、テストでも本前提を
    /// 揃えてからコマンドを呼ぶ。
    fn seed_submit_workflow_log(
        data_dir: &Path,
        run_id: &str,
        worktree_path: &str,
        step_name: &str,
        contract: &str,
    ) {
        let workflow = crate::adaptor::gateway::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: test_schemas(),
            nodes: vec![crate::adaptor::gateway::workflow::schema::NodeDefinition {
                name: step_name.to_string(),
                kind: crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                artifact: Some(contract.to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            data_dir,
            &make_run(run_id, worktree_path, RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(data_dir);
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: worktree_path.to_string(),
            workflow_definition: workflow,
            timestamp: 100.0,
        })
        .unwrap();
    }

    /// [08] CLI 完了基準境界: `submit` は受理キュー投入（pending file の書き出し）まで
    /// 完了した時点で `Ok(())` を返す。書き出された pending entry は
    /// `PendingCommandPayload::SubmitOutput` shape を持ち、`run_id` と `step` /
    /// `contract` / structured_output が永続化される（spec [08] CLI 完了基準）。
    #[test]
    fn cmd_output_submit_writes_pending_file_with_typed_payload() {
        use crate::adaptor::gateway::workflow::pending_command::PendingCommandPayload;
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(91);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-pending",
            "review",
            "review-verdict",
        );
        let json = "{\"verdict\":\"LGTM\"}";
        cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some(json.to_string()),
            None,
        )
        .unwrap();
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0].command.payload {
            PendingCommandPayload::SubmitOutput {
                step_name,
                contract,
                structured_output,
            } => {
                assert_eq!(step_name, "review");
                assert_eq!(contract, "review-verdict");
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("expected SubmitOutput payload, got: {other:?}"),
        }
    }

    /// [08] CLI 完了基準境界: `--file` 入力でも pending file が書き出される。
    #[test]
    fn cmd_output_submit_writes_pending_file_from_file_arg() {
        use crate::adaptor::gateway::workflow::pending_command::PendingCommandPayload;
        let tmp = TempDir::new().unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"verdict\":\"LGTM\"}").unwrap();
        let run_id = test_uuid(92);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-pending-file",
            "review",
            "review-verdict",
        );
        cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            None,
            Some(input_file),
        )
        .unwrap();
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].command.payload,
            PendingCommandPayload::SubmitOutput { .. }
        ));
    }

    /// [08] CLI 入力境界: `--json` も `--file` も指定されない場合は関数レベルで reject。
    #[test]
    fn cmd_output_submit_rejects_missing_json_and_file() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(93);
        let err = cmd_output_submit(tmp.path(), &run_id, "review", "review-verdict", None, None)
            .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] CLI 入力境界: `--json` に invalid JSON が渡されたら InvalidInput を返す。
    #[test]
    fn cmd_output_submit_rejects_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(94);
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some("{not json}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] 振る舞い定義 Rule 1: 対象 run が存在しなければ pending file を作らず CliError。
    /// CLI 同期検証の reject 経路を担保する（spec [08] 副作用なし境界）。
    #[test]
    fn cmd_output_submit_rejects_unknown_run_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(101);
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some("{\"verdict\":\"LGTM\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)));
        // pending file が作られていない
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: workflow に存在しない step に対する submit は
    /// pending file を作らず CliError::InvalidInput を返す。
    #[test]
    fn cmd_output_submit_rejects_unknown_step_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(102);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-unknown-step",
            "review",
            "review-verdict",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "no-such-step",
            "review-verdict",
            Some("{\"verdict\":\"LGTM\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: caller の `--type` が step の expected contract と
    /// 一致しない場合は pending file を作らず CliError::InvalidInput。
    #[test]
    fn cmd_output_submit_rejects_contract_type_mismatch_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(103);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-type-mismatch",
            "review",
            "review-verdict",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "fix-result",
            Some("{\"status\":\"FIXED\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: contract に適合しない JSON は pending file を作らず CliError。
    #[test]
    fn cmd_output_submit_rejects_contract_violation_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(104);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-violation",
            "review",
            "spec-directory",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "spec-directory",
            Some("{}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cmd_output_submit_rejects_spec_dir_outside_repo_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(105);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-path-violation",
            "review",
            "spec-directory",
        );

        for spec_dir in ["/tmp/spec", "../outside"] {
            let err = cmd_output_submit(
                tmp.path(),
                &run_id,
                "review",
                "spec-directory",
                Some(format!(r#"{{"spec_dir":"{spec_dir}"}}"#)),
                None,
            )
            .unwrap_err();
            assert!(matches!(err, CliError::InvalidInput(_)));
            assert!(PendingCommandStore::new(tmp.path())
                .list_pending()
                .unwrap()
                .is_empty());
        }
    }

    /// [08] 振る舞い定義 Rule 2: validate は pure validator を呼び、副作用を起こさない。
    /// pending dir / event log / run file / 既存 step_outputs のいずれも変化しない。
    #[test]
    fn cmd_output_validate_returns_ok_for_contract_compliant_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(95);
        // RunStarted event に workflow definition を埋め込む
        let yaml = crate::adaptor::gateway::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: test_schemas(),
            nodes: vec![crate::adaptor::gateway::workflow::schema::NodeDefinition {
                name: "review".to_string(),
                kind: crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                artifact: Some("spec-directory".to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/validate", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/validate".to_string(),
            workflow_definition: yaml,
            timestamp: 100.0,
        })
        .unwrap();
        // 既存の ArtifactProduced を 1 件入れておき、validate がそれを変化させないことも確認する。
        log.append(&WorkflowEvent::ArtifactProduced {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: Some("review-verdict".to_string()),
            value: serde_json::json!({"verdict": "LGTM"}),
            request_id: Some("00000000-0000-0000-0000-0000000000aa".to_string()),
            submitted_at: Some(120.0),
            timestamp: 130.0,
        })
        .unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"spec_dir\":\"docs/specs/issues-123\"}").unwrap();

        let run_file_path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{run_id}.json"));
        let event_log_before = log.read_log(&run_id).unwrap();
        let run_file_before = std::fs::read_to_string(&run_file_path).unwrap();

        cmd_output_validate(tmp.path(), &run_id, "review", &input_file).unwrap();

        // pending dir 不変
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
        // event log の長さ・内容が validate 前後で一致
        let event_log_after = log.read_log(&run_id).unwrap();
        assert_eq!(event_log_before.len(), event_log_after.len());
        // run file の中身が変わっていない
        let run_file_after = std::fs::read_to_string(&run_file_path).unwrap();
        assert_eq!(run_file_before, run_file_after);
        // 既存の ArtifactProduced がそのまま残る（reconstruct で step_outputs slot が不変）
        let view = build_output_get_view(read_domain_log(tmp.path(), &run_id).unwrap(), "review");
        assert!(matches!(
            view,
            OutputGetView::Submitted {
                ref contract,
                submitted_at: Some(120.0),
                timestamp: 130.0,
                ..
            } if contract == "review-verdict"
        ));
    }

    #[test]
    fn cmd_output_validate_returns_err_for_invalid_contract_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(96);
        let workflow = crate::adaptor::gateway::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: test_schemas(),
            nodes: vec![crate::adaptor::gateway::workflow::schema::NodeDefinition {
                name: "review".to_string(),
                kind: crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                artifact: Some("spec-directory".to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/validate-fail", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/validate-fail".to_string(),
            workflow_definition: workflow,
            timestamp: 100.0,
        })
        .unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"unexpected\":\"value\"}").unwrap();

        let err = cmd_output_validate(tmp.path(), &run_id, "review", &input_file).unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    #[test]
    fn cmd_output_validate_rejects_spec_dir_outside_repo() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(106);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/validate-path-violation",
            "review",
            "spec-directory",
        );
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, br#"{"spec_dir":"../outside"}"#).unwrap();

        let err = cmd_output_validate(tmp.path(), &run_id, "review", &input_file).unwrap_err();

        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] 振る舞い定義 Rule 3: `get` は提出済み output と付随メタを返す。
    /// 同 step に対する複数 ArtifactProduced のうち最後のものを採用する。
    #[test]
    fn cmd_output_get_returns_submitted_when_event_present() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(97);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get"))
            .unwrap();
        log.append(&WorkflowEvent::ArtifactProduced {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: Some("review-verdict".to_string()),
            value: serde_json::json!({"verdict": "LGTM"}),
            request_id: Some("req-1".to_string()),
            submitted_at: Some(150.0),
            timestamp: 200.0,
        })
        .unwrap();

        let view = build_output_get_view(read_domain_log(tmp.path(), &run_id).unwrap(), "review");
        assert!(matches!(
            view,
            OutputGetView::Submitted {
                ref contract,
                submitted_at: Some(150.0),
                timestamp: 200.0,
                ..
            } if contract == "review-verdict"
        ));
    }

    /// [08] 振る舞い定義 Rule 3: 未提出 step は `NotSubmitted` を返す。
    #[test]
    fn cmd_output_get_returns_not_submitted_when_no_event() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(98);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-empty", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get-empty"))
            .unwrap();

        let view = build_output_get_view(read_domain_log(tmp.path(), &run_id).unwrap(), "review");
        assert!(matches!(view, OutputGetView::NotSubmitted));
    }

    #[test]
    fn cmd_output_get_returns_not_found_for_missing_run() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(100);

        let err = cmd_output_get(tmp.path(), &run_id, "review", false).unwrap_err();

        assert_eq!(
            err,
            CliError::NotFound(format!("Workflow run not found: {run_id}"))
        );
    }

    /// [08] 振る舞い定義 Rule 3: 同 step に対し複数の ArtifactProduced が記録された場合、
    /// 最後 (= 最新) の event を採用する。
    #[test]
    fn cmd_output_get_returns_latest_event_when_multiple_submitted() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(99);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-latest", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get-latest"))
            .unwrap();
        log.append(&WorkflowEvent::ArtifactProduced {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: Some("review-verdict".to_string()),
            value: serde_json::json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}),
            request_id: None,
            submitted_at: None,
            timestamp: 110.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::ArtifactProduced {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: Some("review-verdict".to_string()),
            value: serde_json::json!({"verdict": "LGTM"}),
            request_id: None,
            submitted_at: None,
            timestamp: 120.0,
        })
        .unwrap();

        let view = build_output_get_view(read_domain_log(tmp.path(), &run_id).unwrap(), "review");
        match view {
            OutputGetView::Submitted {
                structured_output, ..
            } => {
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("expected Submitted, got: {other:?}"),
        }
    }

    /// [08] 振る舞い定義 Rule 3 (5-5 修正): `output get` は step が workflow に
    /// 存在しない場合、`output validate` と対称に `CliError::InvalidInput` を返す
    /// （exit=2）。`not_submitted` は「step は存在するが未提出」専用。
    #[test]
    fn cmd_output_get_rejects_unknown_step_with_invalid_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(96);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-unknown-step", RunStatus::Running, 100.0),
        );
        // step "review" を持つ workflow を log に播種し、それとは別名の step を問い合わせる。
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/get-unknown-step",
            "review",
            "review-verdict",
        );
        let err =
            cmd_output_get(tmp.path(), &run_id, "nonexistent_step", true).expect_err("must error");
        let CliError::InvalidInput(msg) = &err else {
            panic!("expected CliError::InvalidInput for unknown step, got: {err:?}");
        };
        assert!(
            msg.contains("nonexistent_step"),
            "error message must include step name, got: {msg}"
        );
        assert!(
            msg.contains("artifact"),
            "error message must mention artifact (symmetric with validate), got: {msg}"
        );
    }
}
