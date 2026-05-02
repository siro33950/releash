## 要求

**種別**: 新機能
**ゴール**: Workflow Engineに、step完了後の `StepOutput` を永続化し、複数outputをcollect/reduceして次stepのinput contextと遷移判定に使える基盤を追加する
**背景**: オーケストレーションでは単一の `previous_response` だけでは不十分。並列レビュー・テスト結果集約・UI確認結果・複数Agent出力比較では、複数stepの確定outputを集めて次stepに渡すcontextとrouting判定を作る必要がある。また、stream中のtoken/deltaと確定outputを分離する設計が必要
**利用シーン**: 観点別並列レビュー、テスト結果集約、UI確認結果集約、複数Agent出力の比較
**制約**:
- reducerは宣言型の固定strategy（`last` / `concat` / `grouped` / `any_needs_fix` / `all_passed`）
- 任意スクリプト埋め込みによるreducerは非スコープ
- streaming表示とstep output確定処理は分離する
**影響範囲**: 既存Workflow Engine（step完了処理、prompt composition、遷移判定）、Trace view / 実行ログ

### 受け入れ基準

- auto step完了後、final agent messageが `StepOutput` として保存される
- 次stepが `pass_previous_response` で直前step outputをpromptに受け取れる
- 並列stepの複数outputをcollectできる
- `any_needs_fix` により、1つでもNEEDS_FIXがあれば全体resultがNEEDS_FIXになる
- collect/reduce結果がTrace view / 実行ログから追跡できる
- streaming表示とstep output確定処理が分離されている

### 非スコープ

- 任意スクリプト埋め込みによるreducer
- 汎用artifact永続化基盤の完成
- 高度な型付きoutput schema / evidence schemaの完全実装
- UI動画/スクショ確認の専用artifact化

## 振る舞い定義

```gherkin
Feature: Step Output / Collect / Reduce によるステップ間コンテキスト受け渡し
  Workflow Engineにおいて、step完了後の確定outputを永続化し、
  複数outputをcollect/reduceして次stepのinput contextと遷移判定に使える基盤を提供する

  # ── StepOutput の永続化 ──

  Rule: auto step完了時にfinal agent messageをStepOutputとして保存する
    Scenario: auto stepが正常完了する
      Given auto modeのstepが実行中である
      When agentセッションがturn_completeになる
      Then final agent message textがStepOutputとして保存される
      And StepOutputにstep_name, run_index, session_id, result, token_usage, completed_atが記録される

    Scenario: approval stepが承認される
      Given approval modeのstepが承認待ちである
      When ユーザーがapproveを選択する
      Then 直前のagent message textがStepOutputとして保存される

    Scenario: interactive stepが完了する
      Given interactive modeのstepが実行中である
      When ユーザーがstepを完了させる
      Then 最後のagent message textがStepOutputとして保存される

  # ── 前step output の次step注入（pass_previous_response） ──

  Rule: pass_previous_responseで直前step outputをpromptに受け取る
    Scenario: pass_previous_responseが有効な次stepが開始する
      Given step Aが完了しStepOutputが保存されている
      And step Bがpass_previous_response: trueで定義されている
      When step Bが開始される
      Then step BのpromptにStep AのStepOutput textが注入される

    Scenario: pass_previous_responseが無効な次stepが開始する
      Given step Aが完了しStepOutputが保存されている
      And step Bがpass_previous_responseを指定していない
      When step Bが開始される
      Then step Bのpromptに前step outputは注入されない

  # ── 任意step名指定によるoutput参照（pass_output_from） ──

  Rule: pass_output_fromで任意stepのoutputをpromptに注入する
    Scenario: 単一stepのoutputを参照する
      Given step Aが完了しStepOutputが保存されている
      And step Cがpass_output_from: ["step_a"]で定義されている
      When step Cが開始される
      Then step CのpromptにStep AのStepOutput textが注入される

    Scenario: 複数stepのoutputを参照する
      Given step Aとstep Bが完了しStepOutputが保存されている
      And step Cがpass_output_from: ["step_a", "step_b"]で定義されている
      When step Cが開始される
      Then step CのpromptにStep AとStep BのStepOutput textがstep_name付きで注入される

    Scenario: 参照先stepが未完了である
      Given step Aが未実行である
      And step Bがpass_output_from: ["step_a"]で定義されている
      When step Bが開始される
      Then step_aのoutputがない旨がpromptに反映される

  # ── streaming表示と確定outputの分離 ──

  Rule: streaming中のtokenと確定StepOutputは分離される
    Scenario: streaming中にstepが完了する
      Given auto stepがstreaming中である
      When turn_completeイベントが発生する
      Then StepOutputにはfinal messageの確定テキストが保存される
      And streaming中のdelta tokenはStepOutputに含まれない

  # ── 複数outputのcollect ──

  Rule: 並列stepの複数outputをcollectできる
    Scenario: 並列stepが全て完了する
      Given 並列グループに3つのauto stepが定義されている
      When 3つのstepが全て完了しStepOutputが保存される
      Then 3つのStepOutputがcollect対象として集約される

  # ── reduce strategy ──

  Rule: any_needs_fixは1つでもNEEDS_FIXがあれば全体resultをNEEDS_FIXにする
    Scenario: 1つのstepがNEEDS_FIXを返す
      Given 並列グループの3つのstepのうち2つがLGTM、1つがNEEDS_FIXの結果を持つ
      When any_needs_fix reducerが適用される
      Then 全体resultはNEEDS_FIXになる

    Scenario: 全てのstepがLGTMを返す
      Given 並列グループの3つのstepが全てLGTMの結果を持つ
      When any_needs_fix reducerが適用される
      Then 全体resultはLGTMになる

  Rule: all_passedは全stepがPASSEDの場合のみ全体resultをPASSEDにする
    Scenario: 全てのstepがPASSEDを返す
      Given 並列グループの全stepがPASSEDの結果を持つ
      When all_passed reducerが適用される
      Then 全体resultはPASSEDになる

    Scenario: 1つのstepがFAILEDを返す
      Given 並列グループのstepのうち1つがFAILEDの結果を持つ
      When all_passed reducerが適用される
      Then 全体resultはFAILEDになる

  Rule: last reducerは最後に完了したstep outputのみを結果とする
    Scenario: last reducerが適用される
      Given 複数のStepOutputがcollect対象である
      When last reducerが適用される
      Then 最後に完了したStepOutputのtextのみが次stepに渡される

  Rule: concat reducerは全outputをstep_name付きで連結する
    Scenario: concat reducerが適用される
      Given 複数のStepOutputがcollect対象である
      When concat reducerが適用される
      Then 全StepOutputのtextがstep_name付きで連結され次stepに渡される

  Rule: grouped reducerはresult別にoutputをグルーピングする
    Scenario: grouped reducerが適用される
      Given 複数のStepOutputがcollect対象であり、各々異なるresultを持つ
      When grouped reducerが適用される
      Then StepOutputがresult値ごとにグルーピングされ次stepに渡される

  # ── reduce結果による遷移判定 ──

  Rule: reduce結果がTransitionRuleのmatchパターンとして使用される
    Scenario: reduce結果がNEEDS_FIXで遷移する
      Given 並列グループのreducerがany_needs_fixで全体resultがNEEDS_FIXである
      And 遷移ルールにmatch: NEEDS_FIXが定義されている
      When 遷移判定が行われる
      Then NEEDS_FIXに対応する次stepに遷移する

    Scenario: reduce結果がLGTMで遷移する
      Given 並列グループのreducerがany_needs_fixで全体resultがLGTMである
      And 遷移ルールにmatch: LGTMが定義されている
      When 遷移判定が行われる
      Then LGTMに対応する次stepに遷移する

  # ── Trace view / 実行ログでの追跡 ──

  Rule: collect/reduce結果がTrace viewから追跡できる
    Scenario: 並列stepのcollect/reduce結果がTrace viewに表示される
      Given 並列グループがreduceされ完了している
      When ユーザーがTrace viewを表示する
      Then 各stepのStepOutput結果が表示される
      And reduce後の全体resultが表示される

  Rule: collect/reduce結果が実行ログに記録される
    Scenario: reduce結果がNDJSONログに記録される
      Given 並列グループがreduceされ完了している
      When 実行ログを参照する
      Then 各stepのStepCompletedイベントにoutput textが含まれる
      And reduce結果がログイベントとして記録される
```

## 実装仕様

**対応方針**: 振る舞い定義のStepOutput永続化・pass_previous_response・pass_output_from・collect/reduce基盤を実現するために、既存の`StepHistoryEntry`拡張 + `Step`スキーマ拡張 + prompt末尾へのcontext block自動追加 + reducer型定義・ロジック追加で対応する。並列実行は#862のスコープだが、collect/reduceはシーケンシャル実行でも動作するよう実装する。

**対象コンポーネント**:

| コンポーネント | 変更内容 |
|---|---|
| `state.rs` — `StepHistoryEntry` | `output_text: Option<String>`, `run_index: u32` フィールド追加 |
| `state.rs` — `WorkflowState` | `step_outputs: HashMap<String, StepOutput>` 追加。step_name→最新output のマップ |
| `state.rs` — 新型 `StepOutput` | `step_name: String`, `run_index: u32`, `session_id: Option<String>`, `result: Option<String>`, `output_text: String`, `token_usage: Option<TokenUsage>`, `completed_at: f64` |
| `schema.rs` — `Step` | `prompt` を `Option<StepPrompt>` に変更。`pass_previous_response: Option<bool>`, `pass_output_from: Option<Vec<String>>`, `collect: Option<CollectConfig>` フィールド追加 |
| `schema.rs` — 新型 | `CollectConfig { from: Vec<String>, reduce: ReduceStrategy }`, `ReduceStrategy` enum (`Last` / `Concat` / `Grouped` / `AnyNeedsFix` / `AllPassed`) |
| `validation.rs` | `pass_output_from` / `collect.from` の参照先step名が存在するか検証。`collect` なしのstepは `prompt` 必須。`any_needs_fix` / `all_passed` 使用時に `collect.from` 参照先stepの `rules` 未定義を警告 |
| `engine.rs` — `WorkflowExecution` | `step_outputs: HashMap<String, StepOutput>` フィールド追加 |
| `engine.rs` — `make_step_history_entry` | `output_text: Option<String>` パラメータを受け取り保存。`run_index` を `step_execution_counts` から取得。同時に `step_outputs` を更新 |
| `engine.rs` — `handle_auto_complete` | 抽出済み `text` を `make_step_history_entry` に渡す |
| `engine.rs` — approval/interactive完了 | `extract_last_assistant_output` helperでoutput_textを取得し `make_step_history_entry` に渡す |
| `engine.rs` — `start_step_session` | `resolve_step_prompt` の後に `inject_step_outputs` を呼び、prompt末尾にcontext blockを追加 |
| `engine.rs` — collect step（仮想step） | `apply_advance` / `apply_transition` で遷移先がcollect stepの場合、AgentSessionを起動せずreduceを実行し、reduce結果で遷移判定を行う |
| `engine.rs` — 新型 `ReduceResult` | `strategy: ReduceStrategy`, `result: Option<String>`, `text: String` |
| `log.rs` — `StepCompleted` | `output_text: Option<String>`, `run_index: Option<u32>` フィールド追加 |
| `log.rs` — 新イベント `OutputCollected` | `execution_id: String`, `workflow_name: String`, `step_name: String`, `step_outputs: Vec<CollectedOutputEntry>`, `reduce_strategy: String`, `reduce_result: Option<String>`, `reduce_text: String`, `timestamp: f64` |
| `log.rs` — `reconstruct_state_from_events` | 新フィールド・新イベントの再構築対応 |
| `types/workflow.ts` | `StepHistoryEntry` に `outputText?: string`, `runIndex: number` 追加。`StepOutput`, `CollectConfig`, `ReduceStrategy`, `ReduceResult` 型追加。`WorkflowState` に `stepOutputs` 追加 |
| Trace view UI | 各stepの `outputText` を折りたたみ表示。reduce結果の全体result表示 |

### プロンプト注入の仕組み（context block自動追加方式）

`resolve_step_prompt` で得たプロンプト文字列の後段で `inject_step_outputs` を呼び、prompt末尾にcontext blockを自動追加する。テンプレート変数方式ではなく、`pass_previous_response` / `pass_output_from` が有効な場合にprompt本文とは独立してcontext blockを付与する。これにより、inline prompt / template prompt の種別を問わず動作する。

**注入フォーマット**:
```xml
<step_output name="step_a">
{output_text}
</step_output>
```

**動作**:
- `pass_previous_response: true` → 直前に完了したstepのoutput_textをcontext blockとして末尾に追加
- `pass_output_from: ["step_a", "step_b"]` → 指定stepのoutput_textをそれぞれcontext blockとして末尾に追加
- 参照先stepが未完了の場合 → `<step_output name="step_a">(not yet completed)</step_output>` を追加

### collect stepの実行モデル（仮想step方式）

collect stepはAgentSessionを起動しない仮想stepとして動作する。`Step.prompt` は `Option<StepPrompt>` に変更し、collect stepでは `prompt` 不要とする（`collect` なしのstepは `prompt` 必須をvalidationで保証）。

**フロー**:
1. 前stepが完了し `apply_advance` / `apply_transition` で遷移先が決定される
2. 遷移先stepに `collect` 設定がある場合:
   a. AgentSessionを起動しない
   b. `collect.from` で指定されたstep名のoutputを `step_outputs` から収集
   c. `collect.reduce` に従ってreduceを実行し `ReduceResult` を生成
   d. 遷移判定: `ReduceResult.result` が `Some` ならその値を、`None` なら `ReduceResult.text` を `evaluate_auto_rules` に渡す
   e. collect step自体の `StepHistoryEntry` を記録（result=reduce結果、output_text=reduce後テキスト）
   f. `OutputCollected` ログイベントを記録
   g. 遷移判定結果に基づき次stepへ遷移（ここで初めてAgentSessionが起動される）
3. 遷移先stepに `collect` 設定がない場合 → 従来通りAgentSessionを起動

**`StepOutcome` の拡張**: 既存の `Persist` / `TransitionAndStart` に加えて `ReduceAndTransition` を追加。`execute_outcome` でreduce処理を実行する。

### resultの意味づけ

- `StepOutput.result` → step自身の遷移判定結果（`evaluate_auto_rules` のマッチ結果、approval: `"approve"`/`"reject"` 等）。stepの自律的な判定値
- `ReduceResult.result` → `Option<String>`。reducerの集約結果。`any_needs_fix` / `all_passed` / `last` では `Some` を返し、`concat` / `grouped` では `None` を返す
- 遷移判定: `ReduceResult.result` が `Some` ならその値を `evaluate_auto_rules` に渡す。`None` なら `ReduceResult.text` を渡す

### reducer動作の詳細

| Strategy | 入力 | result算出 | text算出 |
|---|---|---|---|
| `any_needs_fix` | 各stepの `StepOutput.result` | `Some`: 1つでも `NEEDS_FIX` にマッチ → `"NEEDS_FIX"`、全て非マッチ → `"LGTM"` | concat形式（全output連結） |
| `all_passed` | 各stepの `StepOutput.result` | `Some`: 全て `PASSED` にマッチ → `"PASSED"`、それ以外 → `"FAILED"` | concat形式 |
| `last` | 完了順の最後のStepOutput | `Some`: 最後のstepのresultをそのまま（Noneならそのまま） | 最後のstepのoutput_textをそのまま |
| `concat` | 全StepOutput | `None`（遷移判定はtextで行う） | `## step_name\n{output_text}` 形式で連結 |
| `grouped` | 各stepの `StepOutput.result` | `None`（遷移判定はtextで行う） | `## {result}\n- step_a\n- step_c` 形式 |

**source stepのresult保証**:
- `any_needs_fix` / `all_passed` は `StepOutput.result` を優先して使用する
- `result` が `None`（source stepに `rules` 未定義）の場合、フォールバックとして `output_text` に対してreducer固有のregex（`NEEDS_FIX` / `PASSED`）を適用する
- validationで警告: `any_needs_fix` / `all_passed` を使う collect stepの `collect.from` 参照先stepに `rules` が未定義の場合、`log::warn!` を出力する

### approval/interactiveのoutput_text取得

既存の `handle_approval` / `complete_interactive` は `final_parts` を受け取らないため、`extract_last_assistant_output` helperを新設する。

**`extract_last_assistant_output` の動作**:
1. `current_session_id` から `SessionStore.get_session` でChatSessionを取得
2. `messages` を逆順走査し、最後のassistant messageを見つける
3. `parts` からテキストパートを抽出・連結して返す（`extract_text_from_parts` を再利用）
4. セッション未発見/メッセージなしの場合は `None` を返す

### output_textのサイズ制限

- 上限 100KB。超過時はtruncateし末尾に `... (truncated)` を付与

### 検討した代替案

- **テンプレート変数方式によるprompt注入** → 却下。既存の `render_prompt_template` は `PromptTemplate.variables` に定義された変数のみ走査し、inline promptでは変数展開が走らないため、prompt種別を問わず動作するcontext block自動追加方式を採用
- **collectを通常stepとして実装** → 却下。AgentSessionが不要な集約処理のためにセッション起動コストが発生する。仮想step方式で即座にreduce→遷移を行う
- **output_textのregex scanのみによるresult算出** → 却下。step自身の遷移判定結果（`evaluate_auto_rules`のマッチ結果）を優先使用し、`result` が `None` の場合のみフォールバックとしてregex scanする2段構え
- **collect stepのpromptを必須のまま維持** → 却下。使用しないダミーpromptを強制するのは不自然。`prompt: Option<StepPrompt>` に変更し、validationで `collect` なしのstepのみprompt必須とする

### リスク

- output_textが巨大な場合のメモリ・永続化コスト → 100KB上限で緩和
- シーケンシャル版のcollectでは「並列グループ」の概念がないため、collect対象stepは `collect.from` のstep名リスト指定で明示する
- `prompt` の `Option` 化は既存YAMLの後方互換性に影響しない（`Some` のYAMLはそのまま動作）が、既存コードの `step.prompt` 参照箇所で `unwrap` / パターンマッチの修正が必要

### 影響するテスト

- Rust単体テスト:
  - `StepHistoryEntry` / `StepOutput` のserdeテスト（`run_index`, `output_text` 追加）
  - `ReduceStrategy` 各パターン（`any_needs_fix`, `all_passed`, `last`, `concat`, `grouped`）の単体テスト（result `Some`/`None` の分岐含む）
  - `inject_step_outputs` のcontext block生成テスト（`pass_previous_response`, `pass_output_from`, 未完了step参照）
  - `extract_last_assistant_output` のテスト
  - `validation.rs` の `pass_output_from` / `collect.from` 参照先検証テスト、`collect` なしstepのprompt必須テスト、`any_needs_fix`/`all_passed`のrules未定義警告テスト
  - `log.rs` のログ再構築テスト更新（`output_text`, `run_index`, `OutputCollected` イベント対応）
  - `engine.rs` のcollect仮想step → reduce → 遷移フローテスト
  - `schema.rs` の `prompt: Option<StepPrompt>` パーステスト（collect step: promptなし、通常step: promptあり）
- フロントエンド:
  - `workflow.ts` 型追加に伴うTrace view表示テスト
