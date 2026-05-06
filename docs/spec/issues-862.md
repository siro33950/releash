## 要求

**種別**: 新機能
**ゴール**: ワークフロー内で複数のautoステップを同時並列実行し、集約条件で分岐制御する。また、異なるWorktreeで複数のワークフローを同時実行できるようにする。
**背景**: 現在のワークフローエンジンは逐次実行のみ。レビュー等の独立したステップを並列実行することで効率化したい。また、複数のWorktreeで独立したワークフローを同時に走らせるユースケース（例: Worktree Aでfeature開発、Worktree Bでhotfix）に対応する。
**制約**:
- 同一Worktree内での複数ワークフロー並列は対象外
- Phase 2.5（#896 Step Output / Collect / Reduce）完了が前提
- 既存スキーマ・エンジンへの破壊的変更は許容

**影響範囲**:
- ワークフローYAMLスキーマ（`parallel` キー追加、`aggregate` 条件追加）
- ワークフローエンジン（並列ステップのスケジューリング、集約ロジック）
- Worktree管理（ワークフロー単位での自動作成/クリーンアップ）
- UI（並列ステップの可視化）

## 振る舞い定義

```gherkin
Feature: ワークフロー内並列ステップ実行
  ワークフロー定義内で複数のautoステップを同時並列実行し、
  集約条件で次の遷移先を決定する。

  Rule: 並列ステップは同時に実行される
    Scenario: 並列ブロック内の全ステップが同時に開始される
      Given ワークフローに並列ブロック（arch-review, security-review, quality-review）が定義されている
      When ワークフローが並列ブロックに到達する
      Then 3つのステップが同時に実行開始される
      And 各ステップに独立したAgentSessionが生成される

    Scenario: 並列ステップの1つが完了しても他のステップは継続する
      Given 並列ブロックが実行中で3つのステップが動作している
      When arch-reviewステップが完了する
      Then security-reviewとquality-reviewは実行を継続する
      And arch-reviewの出力がStepOutputとして保存される

  Rule: 全並列ステップの完了後に集約条件で遷移が決まる
    Scenario: all_match条件で全ステップが一致する場合
      Given 並列ブロックにaggregate.all_match="LGTM"が定義されている
      And 全並列ステップが完了した
      When 全ステップの結果が"LGTM"である
      Then aggregate.thenで指定されたステップに遷移する

    Scenario: all_match条件で不一致がある場合
      Given 並列ブロックにaggregate.all_match="LGTM"が定義されている
      And 全並列ステップが完了した
      When いずれかのステップの結果が"LGTM"以外である
      Then aggregate.elseで指定されたステップに遷移する

    Scenario: 並列ステップの結果はcollect/reduceで集約される
      Given 並列ブロックの次のステップにcollect設定がある
      And 全並列ステップが完了した
      When 集約ステップに遷移する
      Then 全並列ステップの出力がreduce戦略で集約される
      And 集約結果が次ステップのコンテキストとして渡される

  Rule: 並列ステップ内でエラーが発生した場合はワークフロー全体が失敗する
    Scenario: 並列ステップの1つがエラーで終了した場合
      Given 並列ブロックが実行中である
      When いずれかの並列ステップがエラーで終了する
      Then 他の実行中の並列ステップが中止される
      And ワークフロー全体がFailed状態になる

  Rule: マルチWorktree並列実行
    Scenario: 異なるWorktreeで別々のワークフローを同時実行できる
      Given Worktree Aでワークフロー"implement"が実行中である
      When Worktree Bでワークフロー"review"を開始する
      Then 両方のワークフローが独立して同時に動作する
      And 互いのワークフローに影響を与えない

    Scenario: 同一Worktreeで2つ目のワークフローは開始できない
      Given Worktree Aでワークフローが実行中である
      When Worktree Aで別のワークフローを開始しようとする
      Then エラーとなり開始が拒否される

  Rule: 並列ステップの実行状態表示
    Scenario: 並列ブロック実行中の状態表示
      Given 並列ブロックが実行中である
      When ユーザーがワークフローの状態を確認する
      Then 各並列ステップの個別の進行状態が表示される
      And 並列ブロック全体としての進捗が表示される
```

## 実装仕様

**対応方針**: ワークフロー内並列ステップ実行と集約遷移を実現するために、既存のWorkflow Engineを「単一current step」前提から「parallel block実行中は複数step sessionを追跡できる」状態管理へ拡張する。YAMLスキーマにはIssue #862案に沿って `parallel` と `aggregate` を追加し、各並列子ステップは独立したAgentSessionとして同時起動する。全子ステップ完了後に `aggregate` 条件を評価し、既存の `StepOutput` / `collect` / `reduce` 基盤へ結果を接続する。マルチWorktree並列は既存の `worktree_path -> WorkflowExecution` 管理を維持し、同一Worktree内の2重開始拒否を明示的にテストで保証する。

**対象コンポーネント**:
- `src-tauri/src/workflow/schema.rs`: `Step.mode` を `Option<StepMode>` に変更し、`parallel: Option<Vec<ParallelStep>>` と `aggregate: Option<AggregateConfig>` を追加する。通常stepはvalidationで `mode` 必須、parallel blockは `mode` なし・`parallel` 必須として扱う。既存YAMLは `mode` を持つためファイル移行は不要だが、Rust/TypeScript型とvalidationは破壊的変更として更新する。
- `src-tauri/src/workflow/schema.rs`: `ParallelStep` は通常 `Step` とは別型として定義する。許可するフィールドは `name`, `mode`, `persona`, `policy`, `knowledge`, `instruction`, `output_contract`, `pass_previous_response`, `pass_output_from` のみ。初期スコープでは `mode` は `auto` のみ許可し、`rules` / `cycle_guard` / `collect` / `parallel` / `aggregate` は持たせない。parallel block内のネストparallelは禁止する。
- `src-tauri/src/workflow/schema.rs`: `AggregateConfig` は `all_match: Option<String>`, `any_match: Option<String>`, `then: String`, `else: String` を持つ。`all_match` と `any_match` は排他、どちらか一方を必須、`else` も必須とする。match条件は通常stepの `rules.match` と同じくregexとして評価する。
- `src-tauri/src/workflow/validation.rs`: parallel blockの子step名重複、通常step名との衝突、子stepがauto modeであること、`aggregate.then` / `aggregate.else` の遷移先存在、`aggregate` 条件の妥当性を検証する。`parallel` あり・`aggregate` なしは許可し、全子step完了後に定義順で次stepへadvanceする。`parallel` なし・`aggregate` ありはvalidation errorにする。並列子stepの `pass_output_from` は親parallel blockより前に定義された通常stepまたは過去に完了済みになり得るグローバルstepのみ参照可能とし、同一parallel block内の兄弟子step参照は禁止する。
- `src-tauri/src/workflow/state.rs`: `WorkflowState` に `active_parallel_steps: Vec<ParallelStepState>` を追加する。`ParallelStepState` は公開・serialize用の型で、`step_name`, `state`, `session_id`, `result`, `run_index`, `completed_at` を持つ。`state` は `running` / `completed` / `failed` / `cancelling` の文字列とする。並列実行中、`current_step_index` / `current_step_name` は親parallel blockを指し、`current_session_id` は `None` とする。`step_states` は親parallel blockと子step名の両方をキーに持てるようにし、子stepの出力は `step_outputs["arch-review"]` のように子step名で保存する。
- `src-tauri/src/workflow/engine.rs`: `WorkflowExecution` に `parallel_run: Option<ParallelRunState>` を追加する。`ParallelRunState` は `parent_step_index`, `parent_step_name`, `aggregate`, `children: HashMap<String, ParallelChildRun>`, `cancelling: bool` を持つ。`ParallelChildRun` は `step_name`, `session_id`, `state`, `result`, `output_text` を持つ。
- `src-tauri/src/workflow/engine.rs`: `session_worktree_map: HashMap<String, String>` を `session_workflow_refs: HashMap<String, SessionWorkflowRef>` に置き換える。`SessionWorkflowRef` は `worktree_path`, `logical_step_name`, `kind` を持ち、`kind` は `Parent`, `SequentialStep`, `ParallelChild` のいずれかにする。`on_turn_complete` はこの参照を見て、通常step完了処理または `handle_parallel_child_complete` に分岐する。
- `src-tauri/src/workflow/engine.rs`: parallel block到達時に全子stepのAgentSessionを同時起動する。`StepOutcome` に `StartParallel(WorkflowState)` を追加し、`execute_outcome` で全子stepセッションを起動して `ParallelRunState` と `active_parallel_steps` を更新する。
- `src-tauri/src/workflow/engine.rs`: 並列子stepの1つがエラー終了した場合、同block内の未完了AgentSessionをinterruptし、Workflow全体をFailedにする。
- `src-tauri/src/workflow/engine.rs`: 並列子step完了時は `StepOutput` と `ParallelChildRun` を更新する。未完了の子stepが残る場合は永続化・ブロードキャストのみ行う。全子step完了後、`aggregate.all_match` / `aggregate.any_match` を `StepOutput.result` 優先、未設定時は `output_text` regex fallbackで評価し、`then` / `else` に遷移する。
- `src-tauri/src/workflow/engine.rs`: 下流stepからparallel blockへ再遷移した場合は、親parallel blockの実行回数を `step_execution_counts[parent_step_name]` で増やし、全子stepを再起動する。子stepのrun indexは `step_execution_counts[child_step_name]` でも個別に増やして `StepOutput.run_index` に反映する。サイクルガードは親parallel blockにのみ設定可能とし、子step単位のcycle guardは初期スコープ外とする。
- `src-tauri/src/workflow/engine.rs` / `src-tauri/src/workflow/facet.rs`: `build_step_prompt` は `Step` 固有ではなく共通の prompt source を受け取れるようにする。実装は `PromptStepRef` helperまたは `StepPromptSource` trait相当を導入し、通常 `Step` と `ParallelStep` の両方からfacet参照・pass設定を取り出して既存のfacet合成と `inject_step_outputs` を再利用する。
- `src-tauri/src/workflow/log.rs`: 新規ログイベントとして `ParallelStarted`, `ParallelStepStarted`, `ParallelStepCompleted`, `ParallelCompleted` を追加する。既存 `StepStarted` / `StepCompleted` は後方互換維持のため変更しない。`reconstruct_state_from_events()` は新規parallelイベントから `active_parallel_steps`, 子stepの `StepOutput`, 親blockの完了状態を再構築する。
- `src/types/workflow.ts`: Rust側スキーマと状態追加に合わせて `ParallelStep`, `AggregateConfig`, parallel状態型を追加する。
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx`: 現在の縦タイムラインは維持し、親parallel blockのrow内に子step rowをwrapping gridで表示する。各子stepには状態、run index、session link、result/output previewを表示する。
- `src/remote/hooks/useRemoteWorkflowState.ts` / `src-tauri/src/protocol/workflow.rs`: リモートUIにもparallel状態が同期されるよう型とpayloadを更新する。
- `src-tauri/src/git/worktree.rs` / `src-tauri/src/git/commands.rs`: ワークフロー専用Worktree自動作成が必要な開始APIを追加する場合のみ既存 `create_worktree` を再利用する。ただし初期実装では「選択済みWorktree上でワークフロー開始」を基本経路とし、自動作成は既存Worktree作成UI/APIとの接続に留める。

**YAMLスキーマ例**:

```yaml
steps:
  - name: parallel-review
    parallel:
      - name: arch-review
        mode: auto
        persona: reviewer
        instruction: architecture-review
        output_contract: review-result
      - name: security-review
        mode: auto
        persona: reviewer
        instruction: security-review
        output_contract: review-result
      - name: quality-review
        mode: auto
        persona: reviewer
        instruction: quality-review
        output_contract: review-result
    aggregate:
      all_match: "LGTM"
      then: report
      else: implement

  - name: report
    mode: auto
    instruction: report
    collect:
      from:
        - arch-review
        - security-review
        - quality-review
      reduce: grouped
```

**aggregate と collect/reduce の関係**:
- `aggregate` はparallel block完了直後の分岐判定にのみ使う。
- `collect/reduce` は遷移先stepでの出力集約に使う。parallel blockの分岐先stepが `collect` を持つ場合、`collect.from` には並列子step名を直接指定する。
- 並列子step名はworkflow全体のグローバル名前空間に属する。通常step名との衝突は禁止し、`step_outputs`, `step_history`, `step_states`, `pass_output_from`, `collect.from` は同じ名前解決規則を使う。
- `aggregate` の評価結果自体は `StepOutput` を生成しない。出力集約が必要な場合は遷移先stepで `collect` を定義する。

**技術選定**:
- 新規ライブラリは導入しない。現コードにReact Flowは入っていないため、まず既存 `WorkflowTrace` を拡張してparallel表示に対応する。
- 並列実行は `tokio` と既存AgentSession起動APIを利用する。既存プロセス管理・SessionStore・AgentStatusCenterとの統合を優先する。

**検討した代替案**:
- React Flow / `@xyflow/react` を導入してワークフロー図を全面刷新する案: Issueコメントとは整合するが、現コードには未導入で、#862の中核であるエンジン並列化よりUI刷新のリスクが大きいため今回は採用しない。
- parallel blockを既存の `collect` stepだけで表現する案: collect/reduceは集約には使えるが、複数AgentSessionの同時起動とエラー時一括中止を表現できないため却下する。

**リスク**:
- 既存エンジンは `current_step_index` と `current_session_id` が単一であるため、parallel中の状態管理を局所的に拡張しないと回帰しやすい。並列固有状態は `ParallelRunState` に閉じ込め、通常step遷移に戻る時点で `parallel_run` を `None` に戻す。
- `session_id -> worktree_path` だけでは並列子stepの完了イベントを識別できない。`SessionWorkflowRef` に `logical_step_name` と `kind` を持たせ、完了イベントのルーティングを明確化する。
- aggregate結果の判定が `result` と `output_text` のどちらに依存するか曖昧になりやすい。`StepOutput.result` を優先し、未設定時のみ `output_text` regex fallbackに統一する。
- `Step.mode` のOptional化により、既存コードが `step.mode` を直接matchしている箇所で回帰する可能性がある。通常step accessor/helperを追加し、validation済みの通常stepだけが `mode` を参照する構造にする。
- UI表示はparallel blockの途中状態を扱うため、既存の `step_states` だけでは不足する。親block状態は `step_states[parent]`、子step状態は `active_parallel_steps` と `step_states[child]` の双方で表現する。

**影響するテスト**:
- Rust unit test: YAMLで `parallel` / `aggregate` がparse・validationできること。
- Rust unit test: `ParallelStep` が許可フィールドのみを受け入れ、`rules` / `collect` / ネストparallelを拒否すること。
- Rust unit test: `AggregateConfig` で `all_match` と `any_match` の同時指定、両方未指定、`else` 未指定を拒否すること。
- Rust unit test: `parallel` あり・`aggregate` なしは全子step完了後に定義順advanceし、`parallel` なし・`aggregate` ありはvalidation errorになること。
- Rust unit test: 並列子step名が通常step名または他の子step名と衝突した場合にvalidation errorになること。
- Rust unit test: 並列子stepの `pass_output_from` が同一parallel block内の兄弟子stepを参照した場合にvalidation errorになること。
- Rust unit test: parallel block到達時に複数子stepの開始状態が生成されること。
- Rust unit test: 全子stepがLGTMの場合 `aggregate.then` に遷移すること。
- Rust unit test: 1つでもNEEDS_FIXの場合 `aggregate.else` または `any_match` 側に遷移すること。
- Rust unit test: `aggregate` 後の遷移先stepが `collect.from` で並列子step名を参照し、reduceできること。
- Rust unit test: 子stepの1つがエラー終了すると他子stepをinterruptしWorkflowがFailedになること。
- Rust unit test: parallel blockへ再遷移すると親blockと各子stepのrun indexが増え、全子stepが再起動されること。
- Rust unit test: `ParallelStarted` / `ParallelStepCompleted` / `ParallelCompleted` から履歴状態を再構築できること。
- Rust unit test: 同一Worktreeでは2つ目のworkflow開始が拒否され、異なるWorktreeでは同時実行できること。
- Frontend test: `WorkflowTrace` がparallel block内の子step進捗を個別表示すること。
- Frontend test: `activeParallelSteps` の `running` / `completed` / `failed` / `cancelling` が表示状態に反映されること。
- Remote hook/type test: `workflow_state_sync` がparallel状態を保持して反映すること。
