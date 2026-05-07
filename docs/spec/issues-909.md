# issues-909: Workflow step output を structured artifact として確定する

## 要求

**種別**: 改善
**ゴール**: Workflow step の output を、assistant message の最後のテキストから暗黙に取得する方式から、`output_contract` に従って `<workflow_output>` ブロックで明示的に提出された structured output を抽出・検証・保存する方式に変更する。会話メッセージは人間向けの説明として残し、step間受け渡し・collect/reduce・Trace表示・後続prompt注入には structured output を正本として使う。
**背景**: 現行方式では (1) Agentの補足・サマリーと成果物が混在する (2) LGTM/NEEDS_FIX等をregex的に抽出する必要がある (3) collect/reduceが文章表現に依存する (4) Trace viewでraw messageとworkflow outputの区別が曖昧 (5) #898のSpec駆動workflowで値の安全な受け渡しに文字列契約では不十分
**制約**: 後方互換不要。#896（Step Output / Collect / Reduce基盤）に依存。#898（Spec駆動workflow）をブロックする。
**影響範囲**: StepOutputモデル、Workflow Engine（extraction/validation/retry）、reducer、prompt injection、Trace view、フロントエンド型定義

## 振る舞い定義

```gherkin
Feature: Structured Workflow Output
  Workflow stepのoutputを、assistant messageから暗黙に取得する方式から、
  output_contractに従ってstructured outputを抽出・検証・保存する方式に変更する。
  output_contractがないstepはStepOutputを生成しない。

  Rule: output_contractがあるstepはstructured outputで完了判定する

    Scenario: valid structured outputでstepが完了する
      Given output_contract "review-verdict" が設定されたauto stepが実行中である
      When agentが有効な <workflow_output type="review-verdict"> ブロックを含むresponseを返す
      Then stepはstructured outputのJSON値をStepOutputとして保存して完了する

    Scenario: output_contractがないstepはStepOutputを生成しない
      Given output_contractが設定されていないstepが実行中である
      When agentがresponseを返しstepが完了する
      Then StepOutputは生成されない

  Rule: workflow_output blockがない・不正な場合はcontract repair retryする

    Scenario: workflow_output blockが存在しない場合
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentのresponseに <workflow_output> blockが含まれない
      Then engineは同じsessionにcontract repair promptを送信する
      And retry回数を記録する

    Scenario: workflow_output blockが複数存在する場合
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentのresponseに <workflow_output> blockが2つ以上含まれる
      Then engineは同じsessionにcontract repair promptを送信する

    Scenario: typeがoutput_contractと一致しない場合
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentが <workflow_output type="fix-result"> を返す
      Then engineは同じsessionにcontract repair promptを送信する

    Scenario: JSON parseに失敗する場合
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentが <workflow_output> 内に不正なJSONを返す
      Then engineは同じsessionにcontract repair promptを送信する

    Scenario: contract-specific validationに失敗する場合
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentが verdict フィールドのないJSONを返す
      Then engineは同じsessionにcontract repair promptを送信する

  Rule: retry上限を超えるとworkflowはfailedになる

    Scenario: retry上限内にvalid outputが得られる
      Given contract repair retryが1回実行された状態である
      When agentが2回目のresponseで有効なstructured outputを返す
      Then stepはそのstructured outputで完了する

    Scenario: retry上限を超える
      Given contract repair retryがmax retry回数（デフォルト2回）実行された状態である
      When agentが再度invalid outputを返す
      Then workflowはcontract violationとしてfailedになる

  Rule: built-in contract validatorはcontract typeごとに検証する

    Scenario: spec-file-path contractのvalid output
      Given output_contract "spec-file-path" が設定されたstepが実行中である
      When agentが {"spec_file_path": "docs/spec/issues-898.md"} を含む workflow_output を返す
      Then structured outputが検証を通過しstepが完了する

    Scenario: review-verdict contractのNEEDS_FIX output
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentが verdict "NEEDS_FIX" と1件以上のfindingsを含む workflow_output を返す
      Then structured outputが検証を通過しstepが完了する
      And StepOutput.resultに "NEEDS_FIX" が設定される

    Scenario: review-verdict contractのLGTM output
      Given output_contract "review-verdict" が設定されたstepが実行中である
      When agentが verdict "LGTM" を含む workflow_output を返す
      Then structured outputが検証を通過しstepが完了する
      And StepOutput.resultに "LGTM" が設定される

    Scenario: fix-result contractのvalid output
      Given output_contract "fix-result" が設定されたstepが実行中である
      When agentが status "FIXED" を含む workflow_output を返す
      Then structured outputが検証を通過しstepが完了する
      And StepOutput.resultに "FIXED" が設定される

  Rule: spec-file-path contractはworkflow variableに反映する

    Scenario: spec_file_pathがworkflow variableに設定される
      Given output_contract "spec-file-path" が設定されたstepが完了する
      When structured outputの spec_file_path が "docs/spec/issues-898.md" である
      Then workflow variable "spec_file_path" に "docs/spec/issues-898.md" が設定される

  Rule: reducerはstructured outputで判定する

    Scenario: any_needs_fixがstructured verdictで判定する
      Given collect.reduce が "any_needs_fix" に設定されたstepがある
      When 集約対象のいずれかのStepOutputが structured_output.verdict = "NEEDS_FIX" を持つ
      Then reduce結果は "NEEDS_FIX" になる

    Scenario: 全てLGTMならany_needs_fixの結果はLGTM
      Given collect.reduce が "any_needs_fix" に設定されたstepがある
      When 集約対象の全StepOutputが structured_output.verdict = "LGTM" を持つ
      Then reduce結果は "LGTM" になる

  Rule: 後続stepへのprompt注入はstructured outputを使う

    Scenario: pass_previous_responseでstructured outputが注入される
      Given 前stepがoutput_contract付きで完了しStepOutputが存在する
      And 現stepに pass_previous_response: true が設定されている
      When 現stepのpromptが構築される
      Then 前stepのstructured output（JSON）がcontext blockとして注入される

    Scenario: pass_previous_responseで前stepにStepOutputがない場合
      Given 前stepにoutput_contractがなくStepOutputが存在しない
      And 現stepに pass_previous_response: true が設定されている
      When 現stepのpromptが構築される
      Then 何も注入されない

    Scenario: pass_output_fromでstructured outputが注入される
      Given step "review" がoutput_contract付きで完了しStepOutputが存在する
      And 現stepに pass_output_from: ["review"] が設定されている
      When 現stepのpromptが構築される
      Then "review" stepのstructured output（JSON）がcontext blockとして注入される

    Scenario: workflow variablesがpromptに注入される
      Given workflow variable "spec_file_path" が設定されている
      When stepのpromptが構築される
      Then <workflow_variables> blockとしてspec_file_pathが注入される

  Rule: Trace viewでraw messageとstructured outputを区別して表示する

    Scenario: 完了stepのstructured outputがTrace viewに表示される
      Given output_contract付きstepが完了した状態である
      When ユーザーがTrace viewを表示する
      Then structured output（JSON）がraw assistant messageと区別して表示される

    Scenario: contract violationとretry attemptがTrace viewに表示される
      Given contract repair retryが発生したstepがある
      When ユーザーがTrace viewを表示する
      Then contract violationの内容とretry attempt回数が表示される

    Scenario: review-verdictのverdictバッジがTrace viewに表示される
      Given review-verdict contractのstepが完了した状態である
      When ユーザーがTrace viewを表示する
      Then verdict（LGTM / NEEDS_FIX）がバッジ形式で表示される

    Scenario: spec-file-pathがclickable pathとしてTrace viewに表示される
      Given spec-file-path contractのstepが完了した状態である
      When ユーザーがTrace viewを表示する
      Then spec_file_pathがクリック可能なリンクとして表示される
```

## 実装仕様

**対応方針**: 振る舞い定義のstructured workflow outputを実現するために、Workflow Engine（Rust側）にoutput抽出・検証・retryの機構を追加し、StepOutputモデルを`output_text`ベースから`structured_output: serde_json::Value`ベースに変更する。フロントエンドはTrace viewにstructured output表示を追加する。

**対象コンポーネント**:

| ファイル | 変更内容 |
|---------|---------|
| `src-tauri/src/workflow/state.rs` | StepOutput構造体: `output_text: String` → `structured_output: Option<serde_json::Value>` + `output_contract: Option<String>`。output_textフィールド削除。StepHistoryEntryに `contract_violations: Option<Vec<ContractViolationRecord>>` 追加。WorkflowStateに `workflow_variables: HashMap<String, String>` 追加 |
| `src-tauri/src/workflow/contract.rs` | **新規**。`<workflow_output>` block抽出パーサー、built-in contract validator（spec-file-path / review-verdict / fix-result）、contract repair prompt生成、ContractViolation / ContractViolationRecord型 |
| `src-tauri/src/workflow/engine.rs` | step完了時に`extract_last_assistant_output` → contract抽出・検証の呼び出し追加。validation失敗時のretryループ（同一sessionへの再送信）。`make_step_history_entry`のoutput_text引数をstructured_output引数に変更。retry count管理。output注入ヘルパー `format_step_output_block` を新設し、`inject_step_outputs`（逐次step用）と `build_parallel_step_prompt`（並列child用）の両方からstructured output JSON形式で注入。workflow_variables注入追加 |
| `src-tauri/src/workflow/engine.rs` (reducer) | `apply_reduce`の全5 ReduceStrategyをstructured output対応に変更（後述「apply_reduceの全ReduceStrategy仕様」参照）。`resolve_step_result`のoutput_text regexフォールバック削除（`result`フィールドのみ参照） |
| `src-tauri/src/workflow/engine.rs` (aggregate) | `evaluate_aggregate`の判定を`output.result`のみに変更。`output_text`フォールバック削除（output_contractなしstepはStepOutputが生成されないため、aggregateは`result`フィールドのみで判定する） |
| `src-tauri/src/workflow/log.rs` | StepCompleted/ParallelStepCompletedに`structured_output`フィールド追加、`output_text`フィールド削除。`ContractRepairRequested { execution_id, workflow_name, step_name, attempt, violation_reason, timestamp }` イベント追加 |
| `src-tauri/src/workflow/mod.rs` | `pub mod contract;` 追加 |
| `src-tauri/src/workflow/builtin.rs` | review-verdict / fix-result / spec-file-path の3 output_contractファセット（prompt注入テンプレート用）をビルトイン登録に追加 |
| `src-tauri/src/workflow/builtin_facets/output_contracts/` | `review-verdict.md` / `fix-result.md` / `spec-file-path.md` を新規作成。各contractが期待する `<workflow_output>` 形式をAgentに指示するテンプレート |
| `src-tauri/src/workflow/builtin/trace-test.yml` | 全ステップの `output_contract: test-report` を `review-verdict` に移行。execute stepの `rules` の `match: DONE` → `match: LGTM` に変更。aggregateの `all_match: DONE` を `all_match: LGTM` に変更（review-verdict contractのresultは LGTM/NEEDS_FIX）。`builtin_facets/output_contracts/test-report.md` は削除 |
| `src-tauri/src/workflow/builtin_facets/instructions/test-step.md` | verdict判定指示を `DONE` → `LGTM` に変更（`verdict: DONE` → `verdict: LGTM`）。output contract参照を `test-report` → `review-verdict` に更新 |
| `src/types/workflow.ts` | StepOutput型: `outputText` → `structuredOutput?: Record<string, unknown>` + `outputContract?: string`。StepHistoryEntry型: `outputText?` → `structuredOutput?` + `contractViolations?` 追加。WorkflowState型に`workflowVariables?: Record<string, string>`。WorkflowLogEvent union型に `contract_repair_requested` variant追加（`{ event: "contract_repair_requested", execution_id, workflow_name, step_name, attempt, violation_reason, timestamp }`）。`step_completed` / `parallel_step_completed` の `output_text?` を `structured_output?` に変更。CollectedOutputEntryの `outputTextLen` を `structuredOutput?` に変更 |
| `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx` | structured output表示コンポーネント追加（JSON表示、verdict badge、clickable path）。contract violation / retry attempt表示 |

**output_contractの2つの役割の分離**:

output_contractはYAMLスキーマ上は1つのフィールドだが、2つの役割を持つ:
1. **ファセット（prompt注入テンプレート）**: `builtin_facets/output_contracts/{key}.md` からテンプレートを読み込み、Agentへの指示としてpromptに含める
2. **validator type**: `contract.rs` の `validate_contract` が参照する検証ルール名

両者は同じキー名で対応する。ファセットは「Agentに何を出力させるか」を指示し、validatorは「Agentの出力が契約を満たすか」を検証する。

**contract.rs の設計**:

```rust
// 抽出結果（type照合はここではしない）
pub enum ExtractionResult {
    Found { type_name: String, json: serde_json::Value },
    NoBlock,
    MultipleBlocks,
    InvalidJson(String),
}

// contract検証結果
pub enum ContractValidationResult {
    Valid { structured_output: serde_json::Value, result: Option<String> },
    Invalid(ContractViolation),
}

pub struct ContractViolation {
    pub reason: String,
    pub details: String,
}

// 永続化用（StepHistoryEntryに記録）
pub struct ContractViolationRecord {
    pub attempt: u32,
    pub reason: String,
    pub details: String,
}

// パーサー（抽出のみ、type照合はしない）
pub fn extract_workflow_output(text: &str) -> ExtractionResult;

// type照合 + contract-specific validation
pub fn validate_contract(
    expected_type: &str,
    extraction: ExtractionResult,
) -> ContractValidationResult;

// repair prompt生成
pub fn build_repair_prompt(contract_type: &str, violation: &ContractViolation) -> String;
```

**apply_reduceの全ReduceStrategy仕様**:

`ReduceResult` の `text: String` フィールドを `structured_output: Option<serde_json::Value>` に変更する。

| Strategy | result | structured_output |
|----------|--------|-------------------|
| Last | 最新StepOutputの`result`を引き継ぐ | 最新StepOutputの`structured_output`をそのまま引き継ぐ |
| Concat | `None` | `[{ "stepName": "s1", "output": <s1の structured_output> }, ...]` の配列JSON |
| Grouped | `None` | `{ "LGTM": ["s1", "s2"], "NEEDS_FIX": ["s3"] }` のようにresultごとのstep名配列JSON |
| AnyNeedsFix | `"NEEDS_FIX"` or `"LGTM"` | `structured_output.verdict` で判定。Concatと同形式の配列JSONを生成 |
| AllPassed | `"PASSED"` or `"FAILED"` | `structured_output.verdict` / `structured_output.status` で判定。Concatと同形式の配列JSONを生成 |

`resolve_step_result` は `output.result` のみ参照し、`output_text` へのregex fallbackを削除する。structured output化によりresultはcontract validatorが設定するため、regexでの推測は不要になる。

**engine.rsのretryフロー**:

1. `on_turn_complete` で auto step完了時、output_contractがあれば:
   a. `extract_last_assistant_output` で全文取得
   b. `extract_workflow_output` でblock抽出
   c. `validate_contract(expected_type, extraction)` でtype照合 + 検証
   d. 成功 → StepOutput確定、step完了
   e. 失敗 → ContractViolationRecordを記録、retry count確認
      - 上限未満 → `build_repair_prompt`で修正依頼を同一sessionに送信、ContractRepairRequestedログ出力、`on_turn_complete`を待つ
      - 上限到達 → workflow failed（contract violation）
2. approval / interactive step完了時も同様にcontract検証を実施

**workflow_variables の設計**:
- `WorkflowState`に`workflow_variables: HashMap<String, String>`を追加
- `spec-file-path` contractの検証成功時に`spec_file_path`をvariablesに設定
- `inject_step_outputs`でvariablesを`<workflow_variables>`ブロックとして注入

**影響するテスト**:

- Rust単体テスト:
  - `contract.rs`: extract_workflow_output（正常/異常各パターン）、validate_contract（3 contract type × valid/invalid + type mismatch）、build_repair_prompt
  - `engine.rs`: step完了時のstructured output保存、retry成功/失敗、workflow_variables設定、inject_step_outputs（structured output注入 / workflow_variables注入）、build_parallel_step_prompt（structured output注入）、evaluate_aggregate（resultのみ判定 / output_textフォールバック不要）、apply_reduce全5 strategy（Last引き継ぎ / Concat配列JSON / Grouped結果JSON / AnyNeedsFix verdict判定 / AllPassed verdict判定）、resolve_step_result（resultのみ / regexフォールバックなし）
  - `state.rs`: StepOutput / StepHistoryEntryのserde往復テスト（contractViolations含む）
  - `log.rs`: StepCompleted（structured_output）、ContractRepairRequestedイベントのシリアライズ
  - `builtin/trace-test.yml`: output_contract移行後のパース・validationテスト
- フロントエンドテスト:
  - WorkflowTrace: structured output表示、verdict badge、contract violation / retry attempt表示
