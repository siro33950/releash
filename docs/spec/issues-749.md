## 要求

**種別**: バグ修正
**現在の挙動**: Workspaceを切り替えると、Agentのモード選択がデフォルトにリセットされる
**期待する挙動**: 各Workspaceが独立したAgentモードを保持し、Workspace切り替え時にそのWorkspaceのモードが復元される
**再現手順**:
1. Agentのモードをデフォルト以外（例: Agentモード）に切り替える
2. 別のWorkspaceに切り替える
3. 元のWorkspaceに戻る
4. Agentのモードがデフォルトにリセットされている
**背景**: Workspace切り替え時にAgentのモード状態が失われるため、毎回モードを再設定する必要がありUXが悪い

## 振る舞い定義

```gherkin
Feature: Agentセッション別モードの保持
  Agentセッションのモード（Code/Ask/Plan/Bypass）がセッションごとに
  独立して保持され、セッション切り替えやターン完了時に正しく維持される

  Rule: モードはAgentセッションごとに独立して保持される
    Scenario: セッション切り替え後にモードが復元される
      Given Session Aでモードを"Plan"に変更している
      And Session Bでモードを"Bypass"に変更している
      When Session Aに切り替える
      Then Session Aのモードが"Plan"である

    Scenario: 新規セッションではデフォルトモードが適用される
      Given 新規セッションを作成する
      Then モードが"Code"である

  Rule: モードセレクターはアクティブセッションのモードを表示する
    Scenario: 切り替え先セッションのモードが表示に反映される
      Given Session Aでモードを"Ask"に変更している
      When Session Aに切り替える
      Then モードセレクターに"Ask"と表示される

  Rule: モードはターン完了後も維持される
    Scenario: ターン完了後にPlanモードが維持される
      Given モードを"Plan"に設定している
      When エージェントのターンが完了する
      Then モードが"Plan"のままである

    Scenario: ターン完了後にBypassモードが維持される
      Given モードを"Bypass"に設定している
      When エージェントのターンが完了する
      Then モードが"Bypass"のままである

    Scenario: ターン完了後にAskモードが維持される
      Given モードを"Ask"に設定している
      When エージェントのターンが完了する
      Then モードが"Ask"のままである

  Rule: Plan承認後はCodeモードに切り替わる
    Scenario: Plan承認後にCodeモードになる
      Given モードを"Plan"に設定している
      When エージェントの計画が承認される
      Then モードが"Code"に切り替わる
```

## 実装仕様

**対応方針**: Agentセッション別のモード保持を実現するために、Rust側のChatSessionに`permission_mode`（ユーザー設定モード）を永続化し、モード管理ロジック（保存・復元・Plan承認後の復元）を全てRust側に集約する。フロントエンドはRustから受け取ったモードを表示・送信するインターフェースに徹する。

**対象コンポーネント**:

Rust側:
- `src-tauri/src/session/mod.rs`: `ChatSession`に`permission_mode: String`フィールド追加（デフォルト`"acceptEdits"`、既存JSON互換のため`#[serde(default)]`）。`SessionSummary`にも`permission_mode`を追加
- `src-tauri/src/session/store.rs`: `update_permission_mode(session_id, mode)`メソッド追加
- `src-tauri/src/agent_sdk.rs`:
  - `set_agent_permission_mode`: Bridgeへの送信に加えて`SessionStore`に永続化
  - `init_agent_sessions`: 各セッションのBridge起動時にセッション固有の`permission_mode`を使用
  - SDK側から`permissionMode: "default"`（Plan承認後）が来た時、Rust側で`permission_mode`に基づいて`resolve_permission_mode`を実行し、Bridgeにrestoredモードを送信＋SessionStoreに保存＋フロントにイベント通知
  - 新規Tauriイベント`agent-permission-mode-changed`を追加し、モード変更をフロントに通知

フロントエンド側:
- `src/hooks/agentChatReducer.ts`: `userPermissionMode`フィールド除去（Rust側に移行）。`permissionMode`はRust側から受け取った表示用の値のみ保持。`SET_USER_PERMISSION_MODE`/`RESTORE_USER_PERMISSION_MODE`アクション除去
- `src/hooks/useAgentChat.ts`: `setPermissionMode`はRustコマンド呼び出しのみ（dispatchしない）。`selectSession`/`initSessions`で`GetSessionResponse`から`permissionMode`を取得してdispatch
- `src/hooks/useAgentSdkListeners.ts`: `handlePermissionModeSync`からフロントエンドのモード復元ロジック除去。`agent-permission-mode-changed`イベントをlistenし、Rustから通知されたモードをdispatchするだけ
- `src/types/session.ts`: `ChatSession`に`permissionMode`フィールド追加

**検討した代替案**:
- フロントエンドstateのみでモード管理（reducerに`sessionPermissionModes`マップを追加）: ロジックがフロント側に残るため却下

**リスク**:
- 既存JSONにpermission_modeフィールドがないセッションファイル: `#[serde(default = "default_permission_mode")]`で`"acceptEdits"`をデフォルト値とすることで後方互換性を確保

**影響するテスト**:
- Rust: `session/` — `permission_mode`の永続化・デフォルト値・後方互換性テスト
- Rust: `agent_sdk` — モード変更・Plan承認後の復元ロジックテスト
- フロントエンド: `agentChatReducer.test.ts` — 除去されたアクションの削除、新しいモード反映ロジックのテスト
- フロントエンド: セッション切り替え時のモード表示テスト
