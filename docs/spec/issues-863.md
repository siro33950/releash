## 要求

**種別**: 新機能
**ゴール**: ワークフロー実行のリアルタイム進行状況をGUIで可視化し、実行ログをNDJSON形式で記録して、過去の実行履歴を閲覧可能にする
**背景**: オーケストレーション機能（Milestone #49）のPhase 3。Phase 2（#860 Workflow Engine Core）で構築されたステートマシンエンジンの実行過程をユーザーが把握・追跡できるようにする。実行中のステップ進行状況のリアルタイム表示、トークン使用量を含むログの永続化、過去の実行履歴の閲覧により、ワークフローの透明性と振り返りを実現する。

**UI構成**:
- AgentChatPanel内を左右分割し、左にタブビュー（既存のチャットUI）、右にワークフローパネルを常時配置
- ワークフロー未実行時はパネルに空状態UI（ワークフロー開始ボタン付き）を表示
- ワークフローパネルにはステップリストではなくワークフロー図（フローチャート的なUI）を表示
- 各ステップノードに実行回数を表示
- ステップを展開すると実行ごとのステータスと結果を閲覧可能
- 各実行をクリックすると、対応するAgentSessionのタブがアクティブになる（1ステップ実行 = 1 AgentSession = 1タブとして既に自動表示されるため、タブのアクティブ化のみ）

**スコープ**:

### ワークフロー操作
- ワークフロービューからワークフロー定義を選択して新規実行を開始
- ワークフロービューから実行中のワークフローを停止（中断）

### リアルタイムモニタリング
- ワークフロー図上で実行中/完了/待機中/失敗の各ステップ状態をリアルタイム更新
- ステップ遷移のリアルタイム反映（AgentStatusCenter / Tauriイベント経由）
- サイクルガードの反復回数表示
- 並列ステップの進捗表示（UI表示のみ、エンジン側の並列実行拡張は対象外）

### 実行ログ
- セッション実行ログのNDJSON記録
- ステップごとの開始/完了/遷移イベント
- トークン使用量の記録・集計
- ログ保存先: app dataディレクトリ

### 履歴閲覧
- ワークフローパネルで過去の実行履歴を閲覧
- ステップごとに実行回数と、実行ごとのステータス・結果を確認可能
- 各実行をクリックで対応するAgentSessionタブをアクティブ化
- トークン使用量の確認

**制約**:
- 既存基盤との統合: `agent_status.rs` のAgentStatusCenterでステップ状態をブロードキャスト
- 配信: Tauriイベント + WebSocket でGUI・リモートアプリに配信
- 永続化: `session/` の既存セッション永続化を拡張
- 依存: #860 Workflow Engine Core（Phase 2完了後、#862と並行可）

**検証方法**:
- ワークフロービューからワークフロー定義を選択して実行を開始できる
- ワークフロービューから実行中のワークフローを停止できる
- ワークフロー実行中にワークフロー図でステップ進行状況がリアルタイム更新される
- 各ステップの実行回数が表示される
- ステップ展開で実行ごとのステータス・結果が閲覧できる
- 各実行クリックで該当AgentSessionのタブがアクティブになる
- 実行完了後、NDJSONログが生成されている
- トークン使用量が記録・集計されている
- 過去のワークフロー実行履歴をワークフローパネルから閲覧できる

## 振る舞い定義

```gherkin
Feature: ワークフロー実行のモニタリングと履歴
  ワークフロー実行のリアルタイム進行状況をGUIで可視化し、
  実行ログをNDJSON形式で記録して、過去の実行履歴を閲覧可能にする

  # ── ワークフロー操作 ──

  Rule: ワークフロービューからワークフローを実行できる
    Scenario: ワークフロー定義を選択して新規実行を開始する
      Given ワークフロー定義が存在する
      When ユーザーがワークフロービューでワークフロー定義を選択して実行する
      Then 選択したワークフローの実行が開始される

  Rule: ワークフロービューから実行中のワークフローを停止できる
    Scenario: 実行中のワークフローを中断する
      Given ワークフローが実行中である
      When ユーザーがワークフロービューで停止を実行する
      Then ワークフローが中断状態になる

  # ── リアルタイムモニタリング（状態遷移） ──

  Rule: ステップの実行状態はリアルタイムに反映される
    Scenario: ワークフロー開始で最初のステップが実行中になる
      Given ワークフローが定義されている
      When ワークフローを実行する
      Then 最初のステップが実行中状態になる
      And 残りのステップは待機中状態になる

    Scenario: ステップ完了で次のステップに遷移する
      Given ワークフローのステップ1が実行中である
      When ステップ1が完了する
      Then ステップ1が完了状態になる
      And ステップ2が実行中状態になる

    Scenario: ステップ失敗でワークフローが失敗状態になる
      Given ワークフローのステップが実行中である
      When ステップがエラーで終了する
      Then そのステップが失敗状態になる
      And ワークフロー全体が失敗状態になる

  Rule: サイクルガードの反復回数が追跡される
    Scenario: ステップが繰り返し実行される
      Given サイクルガード付きのステップが定義されている
      When そのステップが条件により再実行される
      Then そのステップの実行回数が増加する

  # ── リアルタイムモニタリング（表示） ──

  Rule: ワークフローパネルはAgentChatPanel内に常時表示される
    Scenario: ワークフローパネルが常時表示される
      Given ユーザーがAgentChatPanelを表示する
      Then 左側にタブビュー（既存チャットUI）が表示される
      And 右側にワークフローパネルが常時表示される

    Scenario: ワークフロー未実行時に空状態UIが表示される
      Given ワークフローが実行されていない
      When ユーザーがAgentChatPanelを表示する
      Then ワークフローパネルに空状態UIとワークフロー開始ボタンが表示される

    Scenario: ワークフロー実行中にワークフロー図が表示される
      Given ワークフローが実行中である
      When ユーザーがAgentChatPanelを表示する
      Then ワークフローパネルにワークフロー図が表示される

  Rule: ワークフロー図は各ステップの現在の状態を反映する
    Scenario: 各ステップの状態が色分けで表示される
      Given ステップ1が完了し、ステップ2が実行中である
      When ユーザーがワークフロー図を表示する
      Then ステップ1は完了状態で表示される
      And ステップ2は実行中状態で表示される
      And 残りのステップは待機中状態で表示される

    Scenario: 各ステップに実行回数が表示される
      Given ステップが3回実行されている
      When ユーザーがワークフロー図を表示する
      Then そのステップノードに実行回数「3」が表示される

  # Deferred to #862 — 並列ステップのスキーマ拡張と合わせて対応
  Rule: 並列ステップの構造がワークフロー図に表示される
    Scenario: 並列定義されたステップが図上で並列に表示される
      Given ワークフロー定義に並列ステップ群がある
      When ユーザーがワークフロー図を表示する
      Then 並列ステップが分岐・合流の構造で表示される
      And 各ステップの進捗が個別に表示される

  # ── ステップ詳細の展開 ──

  Rule: ステップ展開で実行ごとの詳細が閲覧できる
    Scenario: ステップの実行履歴を展開する
      Given ステップが複数回実行されている
      When ユーザーがそのステップを展開する
      Then 実行ごとのステータスと結果が一覧表示される

  Rule: 実行クリックで対応するAgentSessionタブがアクティブになる
    Scenario: 実行をクリックしてタブを切り替える
      Given ステップの実行一覧が展開されている
      When ユーザーが特定の実行をクリックする
      Then 対応するAgentSessionのタブがアクティブになる

  # ── 実行ログ ──

  Rule: ワークフロー実行ログはNDJSON形式で永続化される
    Scenario: ステップイベントがNDJSONで記録される
      Given ワークフローが実行されている
      When ステップの開始・完了・遷移イベントが発生する
      Then 各イベントがNDJSON形式でログファイルに記録される

    Scenario: トークン使用量がログに含まれる
      Given ワークフローのステップが完了する
      When ステップの完了イベントが記録される
      Then トークン使用量がイベントに含まれる

  # ── 履歴閲覧 ──

  Rule: 過去のワークフロー実行履歴が閲覧できる
    Scenario: 過去の実行を一覧表示する
      Given 過去にワークフローが複数回実行されている
      When ユーザーがワークフローパネルで履歴を表示する
      Then 過去の実行が一覧表示される

    Scenario: 過去の実行のワークフロー図を閲覧する
      Given 過去の実行が一覧表示されている
      When ユーザーが特定の実行を選択する
      Then その実行のワークフロー図と各ステップの状態が表示される

  Rule: トークン使用量が集計・表示される
    Scenario: ワークフロー全体のトークン使用量を確認する
      Given ワークフローの実行が完了している
      When ユーザーが実行の詳細を表示する
      Then ワークフロー全体のトークン使用量の合計が表示される
```

## 実装仕様

**対応方針**: ワークフロー実行のモニタリング・ログ・履歴閲覧を実現するために、Rust側に実行ログ（NDJSON）とWorkflowState型拡張を追加し、フロントエンドにReact Flowベースのワークフローパネルを AgentChatPanel内に組み込む。

**対象コンポーネント**:

### 1. Rust — WorkflowState型拡張 (`src-tauri/src/workflow/state.rs`)
- `WorkflowState` に `step_execution_counts: HashMap<String, u32>` を追加（各ステップの実行回数をフロントに公開）
- `WorkflowState` に `workflow_definition: Workflow` を追加（フロントがフロー図を描画するために必要。ステップ名・遷移ルール・並列構造を含む）
- `StepHistoryEntry` に `session_id: Option<String>` を追加（対応するAgentSessionタブへのリンク用）
- `WorkflowExecution::to_workflow_state()` を更新

### 2. Rust — 実行ログNDJSON記録 (`src-tauri/src/workflow/log.rs` 新規)
- `WorkflowEventLog` 構造体: `app_data_dir/workflow_logs/{execution_id}.ndjson` にイベントを追記
- イベント型 `WorkflowLogEvent`:
  - `workflow_started` — ワークフロー開始
  - `step_started` — ステップ開始（ステップ名、実行回数）
  - `step_completed` — ステップ完了（ステップ名、結果、トークン使用量）
  - `step_failed` — ステップ失敗（ステップ名、理由）
  - `workflow_completed` — ワークフロー正常完了（合計トークン使用量）
  - `workflow_failed` — ワークフロー失敗（理由）
  - `workflow_aborted` — ワークフロー中断
- 各イベントにタイムスタンプ、execution_id、workflow_nameを含む
- エンジンの各状態遷移ポイント（`start_workflow`, `execute_outcome`, `set_execution_state`等）からログを書き込み

### 3. Rust — トークン使用量記録 (`src-tauri/src/workflow/state.rs` + `engine.rs`)
- `StepHistoryEntry` に `token_usage: Option<TokenUsage>` を追加
- `TokenUsage` 構造体: `{ input_tokens: u64, output_tokens: u64 }`
- `turn_complete` メッセージからトークン情報を取得 → `on_turn_complete` 経由でステップ履歴に記録
- `WorkflowState` にワークフロー全体の合計トークン使用量を算出するフィールドを追加
- agent_sdk.rs側でClaude SDKの `result` メッセージからトークン情報を取得してエンジンに渡す

### 4. Rust — 履歴閲覧用Tauriコマンド (`src-tauri/src/workflow/commands.rs`)
- `list_workflow_executions(worktree_path)` — 過去のワークフロー実行一覧取得（セッションのworkflow_stateから抽出）
- `get_workflow_execution_log(execution_id)` — NDJSONログファイルの読み込み

### 5. フロントエンド — ワークフローパネル (`src/components/panels/AgentChatPanel/WorkflowPanel/`)
- `WorkflowPanel.tsx` — メインパネル（React Flowベースのフロー図 + ステップ詳細 + 履歴）
- `WorkflowGraph.tsx` — React Flowでワークフロー定義をノード・エッジに変換して描画
- `StepNode.tsx` — カスタムノード（状態色分け、実行回数バッジ、展開可能）
- `StepDetail.tsx` — 展開時の実行履歴一覧（ステータス・結果・セッションリンク）
- `WorkflowHistory.tsx` — 過去実行の一覧表示
- ノードカラーマッピング:
  - running: blue
  - completed: green
  - waiting: gray
  - failed: red
  - waiting_approval: yellow

### 6. フロントエンド — リアルタイムリスニング (`src/hooks/useWorkflowState.ts` 新規)
- `workflow-state-changed` Tauriイベントをリッスン
- WorkflowStateをReact stateに保持
- AgentChatPanelのレイアウトを左右分割（ResizablePanelGroup）: 左=チャットUI、右=ワークフローパネル
- ワークフローパネルは常時表示。未実行時は空状態UI（ワークフロー開始ボタン付き）を表示

### 7. フロントエンド — セッションタブ連携
- ステップ詳細の各実行クリック → 対応するsession_idのタブをアクティブ化（既存の`setActiveSession`呼び出し）

### 8. リモートアプリ (`src/remote/`)
- WebSocketプロトコル（`src-tauri/src/protocol/mod.rs`）に `workflow_state_sync` メッセージ型を追加
- `WsBroadcaster` からワークフロー状態変更時にブロードキャスト
- リモートアプリではRemoteAppHeaderにワークフロー状態バッジ（ワークフロー名・現在ステップ名・状態色分け）を表示。フルのワークフローパネル（フロー図・履歴）はデスクトップアプリのみ

**技術選定**:
- `@xyflow/react` (React Flow v12): ノードベースのフロー図描画ライブラリ。ステップをノード、遷移をエッジとして表現。カスタムノードでステータス色分け・実行回数表示が可能。ズーム・パン・レイアウト機能を標準提供。

**検討した代替案**:
- カスタムSVG/div実装: 依存ゼロだがノード配置・エッジ描画・ズーム・パンの実装コストが高い。React Flowはこれらを標準提供するため却下。
- D3.js: 柔軟だがReactの宣言的UIとの統合が煩雑なため却下。

**リスク**:
- React Flowバンドルサイズ: `@xyflow/react` は ~100KB gzipped だが、Tauriアプリではローカルバンドルのためネットワーク影響なし。
- トークン使用量取得: Claude SDKの出力フォーマットがバージョンにより変わる可能性 → agent_sdk.rsのパース箇所を1箇所に集約して対応。

**影響するテスト**:
- Rust単体テスト: `workflow/log.rs` のNDJSON書き込み・読み込み、`state.rs` の新フィールド含むシリアライズ、`engine.rs` のトークン情報伝搬
- フロントエンドテスト: `WorkflowPanel` のレンダリング、`useWorkflowState` のイベントリスニング、ステップノードの状態表示
- 既存テスト修正: `engine.rs` の `to_workflow_state()` テスト（新フィールド追加に伴う修正）
