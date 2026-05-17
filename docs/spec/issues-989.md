## 要求

**種別**: バグ修正

**ゴール**: ワークフローエンジンで step が新しい実行を開始する瞬間に `step_outputs` から当該 step のエントリを削除することで、ループで同一 step が再実行される際に前回実行値が残り続ける不具合を解消する。これにより、LLM が前回ターンの `<workflow_output>` をそのまま引用して返した場合でも、Contract 違反が「正常完了（Done）」扱いされず、エンジンの正常系（contract retry → 上限超で Failed）に乗るようにする。

**背景**:
- ループで同一 step が再実行される際、`step_outputs` の前回実行値がエンジンによって明示的に削除されない
- `make_step_history_entry`（`engine.rs` L591-605）は `structured_output.is_some()` の時のみ `step_outputs.insert` を呼ぶため、新しい実行で structured_output が更新されない経路では前回値が残り続ける
- 結果、LLM が前回ターンの `<workflow_output>` ブロックを引用して返してきた場合、contract validation は型・JSON 構造のみを見て Valid を返し、`step_outputs` に同じ値が新規登録される（あるいは前回値がそのまま参照される）
- `evaluate_aggregate` / `pass_output_from` がこの前回値（または引用された同値）を引き、Contract に違反しているのに「正常完了（Done）」扱いになり、aggregate が前回判定で先に進んでしまう
- `output_contract` の retry 機構（`MAX_CONTRACT_RETRIES=2`）は出力の型/JSON構造を見るのみで、内容の新しさ（今ターンで生成されたか）を判定できない
- ファセット（instruction）側で「前回ターン引用禁止」を明文化することは可能だが、LLM の遵守に依存し完全保証できない
- エンジン側で「step 実行開始時に step_outputs を明示クリアする」のが根本対策

**現在の挙動**:
- ループで同一 step が再実行されるとき、`step_outputs` の前回値が消去されないまま新しい実行が始まる
- 新しい実行で structured_output が更新されない（または LLM が前回ターンの `<workflow_output>` を引用する）と、aggregate / pass_output_from が前回値を参照する
- Contract 違反が検出されず、aggregate が前回判定でそのまま先に進んでしまう

**期待する挙動**:
- step が新しい実行を開始する瞬間に、`step_outputs` から当該 step のエントリを削除する
- 並列ブロック実行開始時には、全子 step 名と親ブロック名のエントリを削除する
- これにより、新実行で structured_output が出なければ `step_outputs.get(name)` が None となり、aggregate / pass_output_from が前回値を引けなくなる
- 引用された場合も、step_outputs の前回値が消えているので「新規挿入」が必須となり、エンジンの正常系（contract retry → 上限超で Failed）に乗る

**再現手順**:
1. `auto-review-fix` のような review-fix ループワークフローを実行
2. 1周目で全 LGTM 以外（NEEDS_FIX）になる
3. fix → 2周目の `code_review_parallel` で、LLM が前回ターンの `<workflow_output type="review-verdict">` をそのまま引用して返す
4. エンジンは Valid と判定し、`step_outputs` を「前回と同じ値」で更新
5. aggregate が前回と同じ判定で進む → 実態として「動作しているように見えるが前回値で進んでいる」状態

**影響範囲**:
- `src-tauri/src/workflow/engine.rs` の遷移処理 / 並列開始処理
- 既存ビルトインワークフロー（例: `spec-driven-development.yml`）のループ挙動にも同様の修正が適用される
- 既存テストで step_outputs が永続化されることを前提にしているケースがあれば修正が必要
- `pass_output_from` で同一 step を複数回参照するケースでは、新しい値で参照されるようになる（既存挙動より正しい挙動）

## 振る舞い定義

```gherkin
Feature: ワークフロー step 実行間の出力隔離

  Rule: step が新しい実行を開始すると、その step の前回出力は破棄される
    Scenario: ループで同一 step が再実行される
      Given ある step がループ内に配置されている
      And その step は前回実行で出力を残している
      When エンジンがその step の新しい実行を開始する
      Then その step の前回出力は以後参照できない

  Rule: 並列ブロックが新しい実行を開始すると、ブロック自身と全子 step の前回出力は破棄される
    Scenario: ループで並列ブロックが再実行される
      Given 並列ブロックがループ内に配置されている
      And 並列ブロックと全子 step は前回実行で出力を残している
      When エンジンが並列ブロックの新しい実行を開始する
      Then 並列ブロックの前回出力は以後参照できない
      And 全子 step の前回出力も以後参照できない

  Rule: 新しい実行で出力が更新されない場合、後続処理は前回出力を引き継がない
    Scenario: 新しい実行が出力を残さずに終わる
      Given ある step が前回実行で出力を残している
      When その step の新しい実行が出力を残さずに終わる
      Then 後続の集約判定や出力受け渡しはその step を「出力なし」として扱う

  Rule: 今ターンの新規出力が Contract に違反した場合、エンジンは Done ではなく contract retry に進み、上限超で Failed に遷移する
    Scenario: LLM が前回出力と同値を返し、その値が Contract に違反する
      Given レビュー判定をループで再実行するワークフローがある
      And 前回ターンの判定結果が出力として残っている
      When LLM が今ターンに前回と同じ判定結果を返してくる
      And その値が Contract に違反している
      Then エンジンはその値を今ターンの新規出力として Contract 検証にかける
      And エンジンは Done に進まず contract retry を実行する
      And contract retry の上限を超えるとエンジンは Failed に遷移する

    Scenario: LLM が前回出力と同値を返し、その値が Contract に適合する
      Given レビュー判定をループで再実行するワークフローがある
      And 前回ターンの判定結果が出力として残っている
      When LLM が今ターンに前回と同じ判定結果を返してくる
      And その値が Contract に適合している
      Then エンジンはその値を今ターンの新規出力として扱う
      And 集約は前回判定をそのまま引き継がない
```

## アーキテクチャ概要

### 責務配置
- `workflow/engine.rs::WorkflowExecution`: ワークフロー実行状態（`step_outputs`, `current_step_index`, `step_execution_counts`, `parallel_run` 等）の保持と純粋な状態遷移を担う。step が新しい実行を開始する瞬間の `step_outputs` 破棄もここに置く。
- `workflow/engine.rs::apply_advance` / `apply_transition` (`apply_transition_inner`): 次ステップへの確定（`current_step_index` 更新・`step_execution_counts` 加算）を行う純粋な状態変更点。この境界で対象 step（逐次）または親ブロック名＋全子 step 名（並列）を `step_outputs` から削除する。`StepOutcome` の組み立て前に行うこと。担当しない: AgentSession 起動・永続化・ブロードキャストなどの副作用。
- `workflow/engine.rs::start_step_session_with_deps` / `start_parallel_children`: クリア済みの `step_outputs` スナップショットを取り出してプロンプト合成・AgentSession 起動を行う副作用境界。`step_outputs` のクリア・更新そのものは担当しない。
- `workflow/engine.rs::make_step_history_entry`: 実行完了時に `structured_output.is_some()` のときだけ `step_outputs.insert` を行う既存挙動を維持する。クリア責務は持たない。
- ワークフロー定義 (`Step` 構造体): 並列ブロックの子 step 名一覧の参照元。エンジンはここから子名を取得して一括削除する。

### データ/通信フロー
- 逐次 step の再実行: `apply_advance` または `apply_transition_inner` がロック内で `current_step_index` を新 step に更新するタイミングで、`exec.step_outputs.remove(&new_step_name)` を実行する → `StepOutcome::TransitionAndStart` を返す → `execute_outcome` が `start_step_session` を呼び、クリア後の `step_outputs` スナップショットでプロンプト合成。
- 並列ブロックの再実行: 同じ遷移確定点で、新 step が `is_parallel_block()` ならその `parallel` 配下の全子 step 名 + 親ブロック名（`step.name`）を `step_outputs` から一括削除する → `StepOutcome::StartParallel` を返す → `start_parallel_children` がクリア後のスナップショットを取得して子セッションを起動。
- 新実行が出力を残さずに終わる場合: `make_step_history_entry(..., structured_output=None, ...)` が呼ばれても `step_outputs` は更新されない（既存挙動）。クリア済みなので `step_outputs.get(name)` は `None` を返し、`evaluate_aggregate` / `pass_output_from` / `apply_reduce` / `inject_step_outputs` は当該 step を「出力なし」として扱う。
- 同値が返ってきた場合: クリア済みのため新規 `insert` 扱いとなり、Contract 検証は今ターンの値として走る。Invalid なら既存の contract retry 機構（`MAX_CONTRACT_RETRIES`）に乗る。

### 状態Owner
- `step_outputs: HashMap<String, StepOutput>`: `WorkflowExecution`（engine.rs 内）。本修正で削除タイミングを追加する状態。
- `current_step_index: usize`: `WorkflowExecution`。クリア発火条件となる「step 開始の瞬間」を表す。
- `step_execution_counts: HashMap<String, u32>`: `WorkflowExecution`。ループ回数と整合させて同一ロック内で更新する。
- 並列ブロックの子 step 名一覧: `Step.parallel: Option<Vec<Step>>`（ワークフロー定義）。エンジンは新 step インデックスからこれを参照して子名を取得する。
- `parallel_run: Option<ParallelRunState>`: `WorkflowExecution`。並列実行の動的状態。クリア対象の決定には用いない（定義側の `Step.parallel` を参照する）。

### 境界
- 純粋な遷移ロジック（`apply_advance` / `apply_transition_inner`）と副作用境界（`execute_outcome` 配下の `start_step_session` / `start_parallel_children`）の分離を維持する。`step_outputs` クリアは遷移ロジック側の責務とし、副作用境界には染み出させない。
- ワークフロー開始時の初期 step（`current_step_index = 0` の状態構築時）はクリア対象外。クリアは「遷移によって新しい実行を開始する瞬間」のみを対象にする。
- `make_step_history_entry` 側の挿入ロジックは変更しない。クリアと挿入は別経路で動作し、Contract retry 経路を含むエンジン正常系をそのまま活用する。

### 実装に委ねること
- `step_outputs` クリア処理を切り出すヘルパー関数の有無・名前・配置（`apply_advance` と `apply_transition_inner` でインライン共有か、`WorkflowExecution` のメソッドとして切り出すか）。
- 並列ブロックの子 step 名取得方法（`step.parallel.as_ref().map(|v| v.iter().map(|s| s.name.clone()).collect())` 等の具体形）。
- ロック内で `current_step_index` 更新・`step_execution_counts` 加算・`step_outputs` クリアを行う順序（同一ロック内で完結することのみ要求）。
- ログ出力の有無や粒度（既存 `StepStarted` / `ParallelStarted` ログを変更するかどうか）。
- テストケースの具体的構成（既存テストモジュール内のどこに置くか、フィクスチャ・モック設計、`step_outputs` の状態を直接観察するか `StepOutcome` 経路で観察するか）。
- 既存テストが「再実行後も `step_outputs` に前回値が残る」前提に依存している場合の修正方針（テスト側の期待値を新仕様に合わせる）。
