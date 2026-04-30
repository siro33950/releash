## 要求

**種別**: 新機能
**ゴール**: ワークフロー定義（YAML）に基づきステップを順次実行するステートマシンエンジンを構築する。各ステップは既存のAgentSession（agent_sdk.rs）に委譲し、3つのモード（auto / approval / interactive）に応じた結果判定と遷移制御を行う。
**背景**: Releashのオーケストレーション機能（Milestone #49）のPhase 2として、Phase 1（#859 Workflow Schema & Storage）で定義されたYAMLスキーマを実行可能にするエンジンが必要。計画→実装→レビュー→修正ループといったマルチステップのAIワークフローを自動遷移させることで、人間の介在ポイントを柔軟に制御しつつ品質担保を実現する。
**対象ユーザー**: Releashユーザー（開発者）
**制約**:
  - 1ステップ = 1 AgentSession（既存agent_sdk.rsに委譲）
  - AgentStatusCenterで進行状況をブロードキャスト
  - SessionStoreでワークフロー状態を永続化
  - 同一Worktree内の並列実行はなし（Phase 3 #862で対応）
  - ワークフロー内並列もPhase 3スコープ（本Issueではシーケンシャル遷移のみ）
**影響範囲**:
  - agent_sdk.rs: AgentSessionの開始・終了をエンジンから制御
  - agent_status.rs: ワークフロー進行状況の追加ブロードキャスト
  - session/: ワークフロー実行状態の永続化

### スコープ
1. ステートマシン（ステップ遷移、状態管理、実行中/完了/失敗）
2. ステップ実行 → 既存AgentSessionへの委譲
3. 3モードの実装
   - **auto**: AgentSessionを自動実行し、turn_complete後のテキストからタグを正規表現で検出→ルール照合→次ステップ決定
   - **approval**: AgentSession完了後にGUIボタンで人間が判定（承認/差し戻し/中止）
   - **interactive**: 対話型AgentSessionでユーザーが明示的に完了操作＋結果選択
4. autoモードのタグ検出エンジン（正規表現パース）
5. 遷移ルール評価（match条件→next先解決）
6. サイクルガード（max_iterations、超過時の中止 or 警告）
7. ワークフロー実行のTauriコマンド（開始、中止、状態取得）
8. interactiveモードでのメッセージキューイング（旧#699）

### 検証方法
- autoステップが実行され、タグ検出でルールに基づき次ステップに遷移する
- approvalステップで一時停止し、ユーザーの承認操作で次へ進む
- interactiveステップで対話でき、ユーザーが完了操作で次へ進む
- review→implementのサイクルガードがmax_iterationsで発火する
- ワークフロー中断・再開が正しく動作する

### 依存関係
- #859 Workflow Schema & Storage（Phase 1完了後に着手）

## 振る舞い定義

```gherkin
Feature: ワークフローエンジン
  YAMLで定義されたワークフローのステップを順次実行し、
  モードに応じた結果判定と遷移制御を行うステートマシンエンジン。

  Rule: ワークフローはステップを順次実行し状態遷移する

    Scenario: ワークフロー開始で最初のステップが実行される
      Given ワークフロー "plan-implement-review" が定義されている
      And 最初のステップは "plan"（interactiveモード）である
      When ユーザーがワークフローを開始する
      Then ワークフロー実行状態が "running" になる
      And ステップ "plan" のAgentSessionが開始される

    Scenario: ルールのないステップが完了すると定義順で次のステップに遷移する
      Given ステップ "plan" が実行中である
      And ステップ "plan" にrulesが定義されていない
      When ステップ "plan" が完了する
      Then 定義順で次のステップ "implement" のAgentSessionが開始される

    Scenario: 最後のステップが完了するとワークフローが完了する
      Given ステップ "report"（最後のステップ）が実行中である
      When ステップ "report" が完了する
      Then ワークフロー実行状態が "completed" になる

    Scenario: ステップ実行中にAgentSessionがエラー終了するとワークフローが失敗する
      Given ステップ "implement" が実行中である
      When AgentSessionがエラー終了する
      Then ワークフロー実行状態が "failed" になる

  Rule: autoモードはタグ検出で遷移先を決定する

    Scenario: タグがルールにマッチすると指定先に遷移する
      Given ステップ "review" がautoモードで実行中である
      And ルール match="NEEDS_FIX" next="implement" が定義されている
      When AgentSessionが完了しテキストに "<decision>NEEDS_FIX</decision>" が含まれる
      Then ステップ "implement" のAgentSessionが開始される

    Scenario: タグが別のルールにマッチすると対応する遷移先に遷移する
      Given ステップ "review" がautoモードで実行中である
      And ルール match="LGTM" next="report" が定義されている
      When AgentSessionが完了しテキストに "<decision>LGTM</decision>" が含まれる
      Then ステップ "report" のAgentSessionが開始される

    Scenario: タグが検出されない場合はワークフローが失敗する
      Given ステップ "review" がautoモードで実行中である
      And ルールが定義されている
      When AgentSessionが完了しテキストにマッチするタグが含まれない
      Then ワークフロー実行状態が "failed" になる

  Rule: approvalモードは人間の判定で遷移先を決定する

    Scenario: AgentSession完了後に承認待ちで一時停止する
      Given ステップ "report" がapprovalモードで実行中である
      When AgentSessionが完了する
      Then ワークフロー実行状態が "waiting_approval" になる

    Scenario: 承認されると次のステップに遷移する
      Given ステップ "report" がapprovalモードで承認待ちである
      When ユーザーが「承認」を選択する
      Then 次のステップに遷移する

    Scenario: 差し戻されるとルールで指定されたステップに遷移する
      Given ステップ "report" がapprovalモードで承認待ちである
      And ルール match="reject" next="implement" が定義されている
      When ユーザーが「差し戻し」を選択する
      Then ステップ "implement" のAgentSessionが開始される

    Scenario: 中止されるとワークフローが中止になる
      Given ステップ "report" がapprovalモードで承認待ちである
      When ユーザーが「中止」を選択する
      Then ワークフロー実行状態が "aborted" になる

  Rule: interactiveモードはユーザーとの対話後に完了または中止する

    Scenario: ユーザーがメッセージを送信するとAgentSessionに送られる
      Given ステップ "plan" がinteractiveモードで実行中である
      When ユーザーがメッセージを送信する
      Then AgentSessionにメッセージがキューイングされ処理される

    Scenario: ユーザーが完了操作を行うと次のステップに遷移する
      Given ステップ "plan" がinteractiveモードで実行中である
      When ユーザーが「完了」を選択する
      Then 次のステップに遷移する

    Scenario: ユーザーが中止操作を行うとワークフローが中止になる
      Given ステップ "plan" がinteractiveモードで実行中である
      When ユーザーが「中止」を選択する
      Then ワークフロー実行状態が "aborted" になる

  Rule: サイクルガードは同一ステップの無限ループを防止する

    Scenario: max_iterations到達でワークフローが失敗する
      Given ステップ "review" にcycle_guard max_iterations=3 が設定されている
      And "review" ステップが3回実行済みである
      When ステップ "review" に再び遷移しようとする
      Then ワークフロー実行状態が "failed" になる
      And サイクルガード超過が原因として記録される

  Rule: 実行中のワークフローはユーザー操作で中断できる

    Scenario: ユーザーがワークフローを中断する
      Given ワークフローが実行中である
      When ユーザーがワークフローの中断を要求する
      Then 実行中のAgentSessionが中断される
      And ワークフロー実行状態が "aborted" になる

  Rule: ワークフロー実行状態はセッションに永続化される

    Scenario: ステップ遷移時に実行状態が保存される
      Given ワークフローが実行中である
      When ステップが遷移する
      Then 現在のステップ名と実行履歴がセッションに保存される

  Rule: ワークフロー進行状況はリアルタイムでブロードキャストされる

    Scenario: ステップ遷移時に進行状況が通知される
      Given ワークフローが実行中である
      When ステップが遷移する
      Then ワークフローのステップ情報がAgentStatusCenterを通じてブロードキャストされる
```

## 実装仕様

**対応方針**: 振る舞い定義のステートマシン・3モード実行・タグ検出・サイクルガードを実現するために、`src-tauri/src/workflow/` に `engine.rs` を新設し、既存の `agent_sdk.rs`（AgentSession委譲）・`agent_status.rs`（進行状況ブロードキャスト）・`session/`（状態永続化）と統合する。

**対象コンポーネント**:
- `src-tauri/src/workflow/engine.rs`（新規）: ステートマシン本体。`WorkflowExecution` 構造体でステップ遷移・状態管理・タグ検出・サイクルガードを実装
- `src-tauri/src/workflow/commands.rs`（変更）: ワークフロー実行のTauriコマンド追加（`start_workflow`, `abort_workflow`, `get_workflow_state`, `approve_workflow_step`, `complete_interactive_step`）
- `src-tauri/src/workflow/mod.rs`（変更）: `engine` モジュール追加
- `src-tauri/src/session/mod.rs`（変更）: `ChatSession` に `workflow_state: Option<WorkflowState>` フィールド追加
- `src-tauri/src/agent_status.rs`（変更）: `SessionStatus` にワークフローステップ情報を追加、`workflow-state-changed` イベント emit
- `src-tauri/src/agent_sdk.rs`（変更）: `turn_complete` 後にワークフローエンジンへのコールバック追加

### ステートマシン設計

**WorkflowExecution 構造体**:
```rust
pub struct WorkflowExecution {
    pub id: String,                                    // 実行ID (UUID)
    pub workflow_name: String,                         // ワークフロー名
    pub state: WorkflowExecutionState,                 // 実行状態
    pub current_step_index: usize,                     // 現在のステップインデックス
    pub current_step_name: String,                     // 現在のステップ名
    pub step_execution_counts: HashMap<String, u32>,   // ステップ名→実行回数（サイクルガード用）
    pub step_history: Vec<StepResult>,                 // 実行履歴
    pub chat_session_id: String,                       // 紐づくChatSession ID
    pub started_at: f64,
    pub updated_at: f64,
}
```

**WorkflowExecutionState enum**:
```rust
pub enum WorkflowExecutionState {
    Running,
    WaitingApproval,
    Completed,
    Failed { reason: String },
    Aborted,
}
```

### ステップ実行フロー

**autoモード**:
1. エンジンが `start_agent_session` → `send_agent_message`（プロンプト送信）
2. `agent-session-state-changed`（turn_complete, exit_code=0）を検知
3. SessionStoreから最終メッセージの `parts` を取得
4. `Text` パートを結合し、rulesの `match` パターンを正規表現で検索
5. マッチしたルールの `next` ステップに遷移 / マッチなしで `Failed`

**approvalモード**:
1. エンジンが `start_agent_session` → `send_agent_message`（プロンプト送信）
2. turn_complete を検知
3. `WorkflowExecutionState::WaitingApproval` に遷移し一時停止
4. フロントエンドがGUIボタンで `approve_workflow_step` Tauriコマンドを呼び出し
5. 承認→次ステップ / 差し戻し→ルールで指定されたステップ / 中止→Aborted

**interactiveモード**:
1. エンジンが `start_agent_session` → `send_agent_message`（プロンプト送信）
2. ユーザーが `send_agent_message` で対話（既存のチャットUIをそのまま利用）
3. ユーザーが `complete_interactive_step` Tauriコマンドで完了/中止を選択
4. 完了→次ステップ / 中止→Aborted

### タグ検出エンジン

- `TransitionRule.match` の値を正規表現パターンとして `regex::Regex` でコンパイル
- turn_complete後のテキスト（`consolidate_parts` 済み `MessagePart::Text` を結合）に対してマッチ
- 複数ルールがマッチした場合は定義順で最初のルールを採用

### サイクルガード

- `step_execution_counts: HashMap<String, u32>` でステップ名ごとの実行回数を追跡
- ステップに遷移する前に `cycle_guard.max_iterations` と比較
- 超過時は `WorkflowExecutionState::Failed { reason: "Cycle guard exceeded..." }` に遷移

### 永続化

- `ChatSession.workflow_state: Option<WorkflowState>` に `WorkflowExecution` のサブセットを保存
- `#[serde(skip_serializing_if = "Option::is_none", default)]` で既存セッションとのJSON互換性を維持
- ステップ遷移のたびに `SessionStore::save_session` で自動保存

### ブロードキャスト

- `SessionStatus` に `workflow_step: Option<String>` と `workflow_state: Option<String>` を追加
- `update_session` 時に `workflow-state-changed` Tauriイベントを emit
- WebSocket: `WsMessage::WorkflowStateSync` variant を追加（リモートクライアント向け）

### turn_complete統合

- `agent_sdk.rs` の `turn_complete` ハンドラー末尾に、ワークフローエンジンへの通知フックを追加
- ワークフロー実行中のセッションの場合のみ発火
- `WorkflowEngine::on_turn_complete(chat_session_id, exit_code, final_parts)` を呼び出し

**影響するテスト**:
- `workflow/engine.rs`: ステートマシン遷移の単体テスト（正常遷移、タグ検出、サイクルガード、エラーケース）
- `workflow/engine.rs`: タグ検出エンジンの単体テスト（マッチ/不一致/複数マッチ）
- `session/mod.rs`: `workflow_state` フィールド付き ChatSession のシリアライズ/デシリアライズのラウンドトリップテスト
- 既存の `agent_sdk.rs` テスト: turn_complete フックの追加による既存動作への影響がないことの確認
