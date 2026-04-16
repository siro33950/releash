## 要求

**種別**: 改善
**ゴール**: Agent ステータス管理を Rust 側に一元化する。`hook_listener.rs` の `POST /hooks/agent` HTTP サーバー（inbound webhook）でステータスを受信していた経路を、`agent_sdk.rs` から直接 AgentStatus を取得・配信する経路に置き換える。`hook_listener.rs` は完全削除。フロントエンドは Session 単位／Workspace 単位のステータスを Rust から読み取るのみとし、状態導出ロジックを一切持たない。
**背景**: 現在 Agent ステータス（Running/Done/Error/Waiting 等）は外部の Claude Code SDK が HTTP で `hook_listener.rs` の `POST /hooks/agent` に送信し、`AgentStatesMap` に保持されている。フロントは `get_agent_states` でこれを取得し、`deriveAgentState` / `deriveActivityStatus` / `aggregateAgentState` 等のロジックで Session/Workspace 単位の状態を導出している。これは CLAUDE.md の「全てのロジックは Rust に実装し、フロントエンドはインターフェースに徹する」原則に違反しており、状態管理が分散して整合性が取りづらい。さらに `agent_sdk.rs` 経由の SDK 出力からも状態が把握できる構造であり、HTTP 経由の二重経路は不要。
**影響範囲**:
- 削除: `src-tauri/src/hook_listener.rs`、`src/lib/agentStateUtils.ts`（aggregateAgentState）
- 新規: `src-tauri/src/agent_status.rs`、`src/hooks/useSessionStatus.ts`、`src/hooks/useWorkspaceStatus.ts`、`src/hooks/useWorkspaceStatuses.ts`
- 変更: `src-tauri/src/agent_sdk.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/session/mod.rs`、`src-tauri/src/mcp/state.rs`、`src-tauri/src/mcp/mod.rs`、`src-tauri/src/mcp/server.rs`、`src-tauri/src/ws_server/session.rs`、`src/hooks/useAgentChat.ts`、`src/hooks/useWorktreeList.ts`、`src/hooks/useWorkspaceNavigation.ts`、`src/screens/useWorktreeState.tsx`、`src/components/panels/TerminalTabPanel.tsx`、`src/components/workspace/WorkspaceList.tsx`
- 維持（変更しない）: `src-tauri/src/webhook.rs`（Slack/Discord 送信機能）、`src/types/webhook.ts`、`src/hooks/useWebhookConfig.ts`、`src/components/panels/SettingsModal.tsx` の Webhook UI、`NotifySection` 構造体および `update_webhook_url`／`get_notify_config`／`update_notify_config` コマンド

**スコープ外**:
- Slack/Discord 送信機能の改廃
- 新規通知配信先（OS 通知等）の追加
- AgentChat 内 system_notification の追加（必要なら別 Spec で扱う）

## 振る舞い定義

```gherkin
Feature: Agent ステータスの Rust 中央管理

  Rule: ステータスは Rust 中央管理から取得される
    Scenario: フロントが Session のステータスを取得する
      Given AgentProcess が動作している ChatSession が存在する
      When フロントが get_session_status を呼び出す
      Then Rust から SessionStatus（agent_state, turn_phase, session_state, pending_permission, last_activity_at）が返る

    Scenario: フロントが Workspace のステータスを取得する
      Given 1 つの worktree に複数の AgentProcess が動作している
      When フロントが get_workspace_status を呼び出す
      Then 配下 SessionStatus を集約した WorkspaceStatus（aggregated_state, running_count, waiting_count, error_count, last_activity_at）が返る

    Scenario: フロントが全 Workspace のステータスを取得する
      Given 複数の worktree が登録されている
      When フロントが list_workspace_statuses を呼び出す
      Then 全 worktree の WorkspaceStatus 配列が返る

  Rule: ステータス変化は Rust から emit される
    Scenario: AgentProcess の状態が変化する
      When AgentProcess の turn_phase または session_state が変化する
      Then session-status-changed イベントが SessionStatus を payload として emit される
      And 対応する WorkspaceStatus が更新されたら workspace-status-changed イベントが WorkspaceStatus を payload として emit される

    Scenario: モバイル remote へのブロードキャスト
      When SessionStatus または WorkspaceStatus が更新される
      Then 互換のため WsMessage::AgentStateSync が WebSocket でブロードキャストされる

  Rule: Workspace 集約ルール
    Scenario: 配下に Error がある
      Given Workspace 配下の Session のうち 1 つが Error 状態である
      Then WorkspaceStatus.aggregated_state は Error

    Scenario: Error はないが Waiting がある
      Given Workspace 配下の Session のうち 1 つが Waiting 状態である
      And Error 状態の Session はない
      Then WorkspaceStatus.aggregated_state は Waiting

    Scenario: Error/Waiting はないが Running がある
      Given Workspace 配下の Session のうち 1 つが Running 状態である
      And Error/Waiting 状態の Session はない
      Then WorkspaceStatus.aggregated_state は Running

    Scenario: 全 Session が Done
      Given Workspace 配下の全 Session が Done 状態である
      Then WorkspaceStatus.aggregated_state は Done

  Rule: フロントエンドは状態導出ロジックを持たない
    Scenario: AgentChat の状態表示
      When AgentChat が表示される
      Then useSessionStatus フックで取得した SessionStatus.agent_state をそのまま表示する
      And フロント側で turn_phase や session_state から状態を再導出しない

    Scenario: WorkspaceList の状態表示
      When WorkspaceList が表示される
      Then useWorkspaceStatuses フックで取得した WorkspaceStatus.aggregated_state をそのまま表示する
      And フロント側で配下 Session を集約しない

  Rule: 表示は通知設定と独立している
    Scenario: 通知設定が無効でも表示は通常通り
      Given NotifySection の on_running が false に設定されている
      When AgentProcess が Running に遷移する
      Then AgentChat と WorkspaceList の表示は通常通り更新される

    Scenario: 表示は webhook_url 設定の有無に依存しない
      Given webhook_url が空文字である
      When AgentProcess が状態遷移する
      Then AgentChat と WorkspaceList の表示は通常通り更新される

  Rule: Webhook 外部送信機能は維持される
    Scenario: webhook_url 設定で Slack/Discord 通知が送信される
      Given webhook_url が設定されている
      And NotifySection の on_running が true に設定されている
      When AgentProcess が Running に遷移する
      Then 設定された webhook_url に Slack/Discord 形式のペイロードが POST される

    Scenario: 通知設定が無効なら Webhook 送信されない
      Given webhook_url が設定されている
      And NotifySection の on_running が false に設定されている
      When AgentProcess が Running に遷移する
      Then Webhook 送信は発火しない

    Scenario: webhook_url 未設定なら Webhook 送信されない
      Given webhook_url が空文字である
      When AgentProcess が状態遷移する
      Then Webhook 送信は発火しない

    Scenario: WhenInactive モードでアプリがアクティブなら送信されない
      Given desktop_mode が WhenInactive に設定されている
      And アプリがアクティブである
      When AgentProcess が状態遷移する
      Then Webhook 送信は発火しない

    Scenario: WhenInactive モードでも非アクティブタイムアウト超過時は送信される
      Given desktop_mode が WhenInactive に設定されている
      And アプリが非アクティブタイムアウトを超過している
      When AgentProcess が状態遷移する
      And 該当状態の通知フラグが有効である
      Then Webhook 送信が発火する

  Rule: 同一状態への遷移ではブロードキャスト・通知が抑止される
    Scenario: 同じ状態のイベントが連続する
      Given AgentProcess が Running 状態である
      When 再度 Running 状態への遷移イベントが発生する
      Then session-status-changed / workspace-status-changed は emit されない
      And Webhook 送信も発火しない

  Rule: hook_listener.rs (POST /hooks/agent) は廃止される
    Scenario: HTTP エンドポイントは存在しない
      When POST /hooks/agent に対してリクエストが送信される
      Then Connection refused または該当エンドポイントが応答しない

    Scenario: AgentStatesMap は廃止される
      When フロントが get_agent_states を呼び出す
      Then 該当コマンドは存在しない（list_workspace_statuses 等に置き換わっている）
```

## 実装仕様

**対応方針**: Agent ステータスの唯一のソースを `agent_sdk.rs` の `AgentProcessMap` に集約し、その状態遷移をフックに `AgentStatusCenter` が SessionStatus / WorkspaceStatus を導出・配信する構造に変更する。`hook_listener.rs` は完全削除。Webhook 送信（Slack/Discord）は `agent_sdk.rs` の状態遷移時に `webhook.rs` を呼び出す形で維持し、`NotifySection.should_notify` 相当の判定もそこで行う。フロントエンドは新フック `useSessionStatus` / `useWorkspaceStatus` / `useWorkspaceStatuses` から Rust の Status を読み取るのみとし、`deriveAgentState` / `deriveActivityStatus` / `aggregateAgentState` 等のロジックを削除する。

### 新規: `src-tauri/src/agent_status.rs`（ステータス中央管理）

```rust
pub struct AgentStatusCenter {
    sessions: Arc<RwLock<HashMap<String /* chat_session_id */, SessionStatus>>>,
    workspaces: Arc<RwLock<HashMap<String /* worktree_id */, WorkspaceStatus>>>,
    app_handle: tauri::AppHandle,
    broadcaster: Arc<WsBroadcaster>,
}

#[derive(Clone, Serialize)]
pub struct SessionStatus {
    pub chat_session_id: String,
    pub worktree_id: String,
    pub worktree_path: String,
    pub pty_id: Option<String>,
    pub agent_state: AgentState,        // turn_phase + session_state から導出
    pub turn_phase: TurnPhase,
    pub session_state: SessionState,
    pub pending_permission: bool,
    pub last_activity_at: i64,          // unix epoch ms
}

#[derive(Clone, Serialize)]
pub struct WorkspaceStatus {
    pub worktree_id: String,
    pub worktree_path: String,
    pub aggregated_state: AgentState,   // 配下 SessionStatus を集約
    pub running_count: usize,
    pub waiting_count: usize,
    pub error_count: usize,
    pub session_count: usize,
    pub last_activity_at: i64,
}
```

**集約ルール**（`AgentStatusCenter::aggregate`）:
1. 配下 Session に 1つでも `Error` → `Error`
2. 1つでも `Waiting` → `Waiting`
3. 1つでも `Running` → `Running`
4. それ以外 → `Done`

**SessionStatus.agent_state 導出**（`AgentStatusCenter::derive_agent_state`、現行フロントの `deriveAgentState` を Rust 移植）:
- `turn_phase == WaitingPermission` → `Waiting`
- `session_state == Error` → `Error`
- `turn_phase == Streaming` または `session_state == Active` → `Running`
- それ以外 → `Done`

**更新トリガー**（呼び出し側が `AgentStatusCenter::update_session(...)` を叩く）:
- `agent_sdk.rs::start_agent_turn` — turn_phase が Streaming に遷移
- `agent_sdk.rs::accumulate_sdk_message` — turn_phase 変化、SDK message 受信
- `agent_sdk.rs::turn_complete` — turn_phase が Idle、session_state が Done/Error
- `agent_sdk.rs` の permission 受付/解放 — pending_permission, turn_phase=WaitingPermission
- `agent_sdk.rs` の AgentProcess 開始/終了 — エントリ追加/削除
- `session/mod.rs` の ChatSession 状態変更 — session_state

**dedup**: `update_session` は前回の SessionStatus と等価な場合は何もしない（`agent_state` / `turn_phase` / `session_state` / `pending_permission` のすべてが同一なら早期 return）。

**emit**: 更新があった場合のみ:
- `app_handle.emit("session-status-changed", &session_status)`
- 影響を受けた WorkspaceStatus を再計算し、変化があれば `app_handle.emit("workspace-status-changed", &workspace_status)`
- 互換のため `broadcaster.try_send(WsMessage::AgentStateSync(...))` も並行送信

**Tauri コマンド**:
- `get_session_status(chat_session_id: String) -> Option<SessionStatus>`
- `get_workspace_status(worktree_id: String) -> Option<WorkspaceStatus>`
- `list_workspace_statuses() -> Vec<WorkspaceStatus>`

### 削除: `src-tauri/src/hook_listener.rs`

ファイル全体を削除。内訳と移管先:

| 旧機能 | 移管先 |
|---|---|
| `start_hook_listener` HTTP サーバー (POST /hooks/agent) | 廃止（agent_sdk.rs から直接） |
| `handle_agent_hook` リクエスト処理 | 廃止 |
| `AgentStatesMap` | `AgentStatusCenter` |
| `get_agent_states` Tauri コマンド | `list_workspace_statuses` に置換 |
| `agent-state-changed` イベント emit | `session-status-changed` / `workspace-status-changed`（互換のため `agent-state-changed` も並行 emit） |
| `WsMessage::AgentStateSync` ブロードキャスト | `agent_sdk.rs` から `WsBroadcaster` 経由 |
| `should_notify` 関数 | `webhook.rs` 内に移動（Webhook 送信判定として本来の用途） |
| repo 自動登録 (`repo_registry::add_repo`) | `agent_sdk.rs` の AgentProcess 起動処理に移管 |
| helpers (`resolve_worktree_root`, `agent_state_key`, `extract_bearer_token`, `normalize_slashes`, `error_response`) | 利用元と一緒に削除 |
| tests | 削除（agent_status.rs / webhook.rs に同等テスト新設） |

### 変更: `src-tauri/src/lib.rs`

- `mod hook_listener;` 削除
- `start_hook_listener` の起動コード削除
- `AgentStatesMap` の `manage` 登録削除
- `hook_listener::get_agent_states` の `invoke_handler` 登録削除
- `mod agent_status;` 追加
- `Arc::new(agent_status::AgentStatusCenter::new(...))` の `manage` 追加
- `agent_status::get_session_status` / `get_workspace_status` / `list_workspace_statuses` を `invoke_handler` に追加
- `server.hook_port` 設定の参照箇所削除（必要なら config からもフィールド削除）

### 変更: `src-tauri/src/agent_sdk.rs`

- 各状態遷移ポイントで `AgentStatusCenter::update_session(...)` を呼び出す
  - `start_agent_turn`、`accumulate_sdk_message`、`turn_complete`、permission 関連、process 起動/終了
- 状態遷移時に Webhook 送信判定を実行:
  ```rust
  if let Ok(cfg) = app_config.get_config() {
      let notify = &cfg.server.notify;
      let url = notify.webhook_url.clone();
      if !url.is_empty() && webhook::should_notify(notify, &agent_state, &focus_tracker) {
          tokio::spawn(async move { webhook::send_webhook(&url, &sync).await; });
      }
  }
  ```
- AgentProcess 起動時に `repo_registry::add_repo` を呼ぶ（hook_listener から移管）

### 変更: `src-tauri/src/webhook.rs`

- `should_notify(notify: &NotifySection, state: &AgentState, focus_tracker: &Mutex<FocusTracker>) -> bool` を `pub` で追加（hook_listener.rs から移植）
- 既存 `send_webhook` はそのまま維持

### 変更: `src-tauri/src/session/mod.rs`

- `ChatSession.state` 変更時に `AgentStatusCenter::update_session(...)` を呼び出す

### 変更: `src-tauri/src/mcp/state.rs`、`mcp/mod.rs`、`mcp/server.rs`、`ws_server/session.rs`

- `AgentStatesMap` 参照を `AgentStatusCenter` 経由（`list_workspace_statuses` 相当）に切り替え
- WS セッション初期データ送信は `AgentStatusCenter` から取得

### 削除: `src/lib/agentStateUtils.ts` および `agentStateUtils.test.ts`

`aggregateAgentState`、`agentStateKey` を削除。集約ロジックは Rust 側で完結する。

### 新規: `src/hooks/useSessionStatus.ts`

```ts
export function useSessionStatus(chatSessionId: string | null): SessionStatus | null {
  const [status, setStatus] = useState<SessionStatus | null>(null);
  useEffect(() => {
    if (!chatSessionId) return;
    invoke<SessionStatus | null>("get_session_status", { chatSessionId }).then(setStatus);
    const unlisten = listen<SessionStatus>("session-status-changed", (event) => {
      if (event.payload.chat_session_id === chatSessionId) setStatus(event.payload);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [chatSessionId]);
  return status;
}
```

### 新規: `src/hooks/useWorkspaceStatus.ts` / `useWorkspaceStatuses.ts`

`get_workspace_status` / `list_workspace_statuses` 呼び出し + `workspace-status-changed` listener。useSessionStatus と同形式。

### 変更: `src/hooks/useAgentChat.ts`

- `deriveAgentState`、`deriveActivityStatus` を削除
- `useSessionStatus(chatSessionId)` で Rust からの SessionStatus を取得し、`agent_state` をそのまま使う
- 既存の `agent-state-changed` listener は段階的に削除（互換期間中は維持しても可）

### 変更: `src/hooks/useWorktreeList.ts`、`useWorkspaceNavigation.ts`、`src/screens/useWorktreeState.tsx`、`src/components/panels/TerminalTabPanel.tsx`、`src/components/workspace/WorkspaceList.tsx`

- `get_agent_states` → `list_workspace_statuses` への切り替え
- `agent-state-changed` 個別合成ロジックを削除し、`useWorkspaceStatuses` の値をそのまま表示
- `agentStatesMap` 状態を削除

### 影響するテスト

**Rust**:
- `hook_listener.rs` テスト全削除
- `agent_status.rs` 新規テスト（集約ルール、derive_agent_state、update_session の dedup、emit、ws broadcast）
- `webhook.rs` の `should_notify` テストを移植（hook_listener.rs から）
- `agent_sdk.rs` 各状態遷移ポイントで `AgentStatusCenter::update_session` が呼ばれるテスト追加
- `agent_sdk.rs` Webhook 送信判定テスト（webhook_url 空 / on_xxx false / WhenInactive ブロック）

**フロントエンド**:
- `agentStateUtils.test.ts` 削除
- `useWorktreeList.test.ts` の `get_agent_states` モック → `list_workspace_statuses` に変更
- `useSessionStatus`、`useWorkspaceStatuses` の単体テスト新規追加（invoke モック + listen モック）

### 段階的移行のための互換維持

- `agent-state-changed` イベントは旧フロントリスナー存続期間中は並行 emit する
- `WsMessage::AgentStateSync` は引き続き broadcast する（モバイル remote の互換性維持）
- これらは将来別 Spec で除却を検討
