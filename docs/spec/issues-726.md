## 要求

**種別**: バグ修正

**問題1: 応答結果の消失**
- **現在の挙動**: AgentChatでエージェントが処理中にWorktreeを切り替えると、エージェントはバックエンドで応答を完了しているが、フロントエンド側でその結果を受け取れず、チャット履歴に反映されない
- **期待する挙動**: Worktreeを切り替えて戻った際に、エージェントの応答結果がチャット履歴に保持・反映されている

**問題2: ストリーミング状態の消失**
- **現在の挙動**: ストリーミング中にWorktree/Sessionを切り替えて戻ると、ストリーミング状態が復元されず、メッセージ送信が可能になり中止操作ができない
- **期待する挙動**: ストリーミング中に切り替えて戻った際、ストリーミング状態が正しく復元され、メッセージ送信が無効化され中止操作が可能である

**再現手順**:
1. AgentChatでエージェントにリクエストを送信する
2. エージェントが処理中の状態でWorktreeを別のものに切り替える
3. 元のWorktreeに戻る
4. AgentChatの結果が反映されていない（エージェントはバックエンドで応答を完了している）
5. ストリーミング中であってもメッセージ送信が可能で、中止ボタンが表示されない

**背景**: フロントエンドのReactコンポーネントがWorktree切り替え時に再マウントされ、reducer stateとストリーミングバッファが破棄されるため、応答結果とストリーミング状態の両方が失われる

## 振る舞い定義

```gherkin
Feature: AgentChatの画面遷移時の結果保持
  AgentChatでエージェントが処理中にWorktreeやSessionを切り替えても、
  完了した応答結果が失われないようにする

  Rule: エージェントの応答はWorktree切り替えを跨いで保持される

    Scenario: エージェント処理中にWorktreeを切り替えて戻ると応答が保持されている
      Given AgentChatでエージェントにリクエストを送信している
      And エージェントが処理中である
      When 別のWorktreeに切り替え、元のWorktreeに戻る
      Then エージェントの応答結果がチャット履歴に反映されている

    Scenario: エージェント処理完了後にWorktreeを切り替えて戻ると応答が保持されている
      Given AgentChatでエージェントが応答を完了している
      When 別のWorktreeに切り替え、元のWorktreeに戻る
      Then エージェントの応答結果がチャット履歴に反映されている

  Rule: エージェントの応答はSession切り替えを跨いで保持される

    Scenario: エージェント処理中にSessionを切り替えて戻ると応答が保持されている
      Given AgentChatでエージェントにリクエストを送信している
      And エージェントが処理中である
      When 別のSessionに切り替え、元のSessionに戻る
      Then エージェントの応答結果がチャット履歴に反映されている

    Scenario: エージェント処理完了後にSessionを切り替えて戻ると応答が保持されている
      Given AgentChatでエージェントが応答を完了している
      When 別のSessionに切り替え、元のSessionに戻る
      Then エージェントの応答結果がチャット履歴に反映されている

  Rule: ストリーミング中の画面遷移ではストリーミング状態が正しく復元される

    Scenario: ストリーミング中にWorktreeを切り替えて戻るとストリーミングが継続表示される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のWorktreeに切り替え、元のWorktreeに戻る
      Then ストリーミング中の応答が継続して表示される

    Scenario: ストリーミング中にSessionを切り替えて戻るとストリーミングが継続表示される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のSessionに切り替え、元のSessionに戻る
      Then ストリーミング中の応答が継続して表示される

    Scenario: ストリーミング中にWorktreeを切り替えて戻るとストリーミング状態が復元される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のWorktreeに切り替え、元のWorktreeに戻る
      Then メッセージ送信が無効化されている
      And ストリーミングの中止操作が可能である

    Scenario: ストリーミング中にSessionを切り替えて戻るとストリーミング状態が復元される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のSessionに切り替え、元のSessionに戻る
      Then メッセージ送信が無効化されている
      And ストリーミングの中止操作が可能である

    Scenario: 別Worktree表示中にストリーミングが完了し戻ると完了状態で表示される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のWorktreeに切り替える
      And 切り替え中にストリーミングが完了する
      And 元のWorktreeに戻る
      Then エージェントの完了済み応答がチャット履歴に反映されている
      And メッセージ送信が可能である
      And ストリーミング中の表示になっていない

    Scenario: 別Session表示中にストリーミングが完了し戻ると完了状態で表示される
      Given AgentChatでエージェントがストリーミング応答中である
      When 別のSessionに切り替える
      And 切り替え中にストリーミングが完了する
      And 元のSessionに戻る
      Then エージェントの完了済み応答がチャット履歴に反映されている
      And メッセージ送信が可能である
      And ストリーミング中の表示になっていない
```

## 実装仕様

**対応方針**: ストリーミング中のコンテンツ蓄積・永続化・状態管理をRustバックエンドに一元化する。フロントエンドからストリーミングのパース・蓄積・永続化ロジックを削除し、バックエンドが提供するデータをそのまま表示する設計に変更する。

### Rust側の変更

**`src-tauri/src/agent_sdk.rs`**:
- `AgentProcess` に `streaming_message_id: Option<String>` と `streaming_parts: Vec<MessagePart>` を追加
- stdoutリーダー内で、現在パススルーしているSDKメッセージを解析し `MessagePart` として蓄積:
  - `stream_event` + `content_block_delta` → Text / Thinking
  - `assistant` + `tool_use` → ToolUse
  - `user` + `tool_result` → ToolResult
  - `permission_request` → Permission
  - `system` (task subtype) → TaskStatus
  - `error` → Error
- 蓄積後、`streaming-message-updated` イベントを発火（スロットル 50-100ms）
- ストリーミング中、1秒間隔で `SessionStore.update_message_parts()` に永続化
- `turn_complete` 時にバッファを最終永続化してクリア
- メタイベント（`permissionMode`同期、`supported_commands`等）は `agent-sdk-message` で引き続きパススルー

**Tauriイベントの変更**:
- `streaming-message-updated`（新規）: `{ chat_session_id, message: ChatMessage }` — 蓄積済みpartsを含む完全なメッセージ。フロントエンドはこのメッセージをそのまま差し替えて表示する
- `agent-streaming-started`（新規）: `{ chat_session_id, message_id }` — ストリーミング開始通知
- `agent-query-completed`（既存維持）: `{ chat_session_id, exit_code, stderr }`
- `agent-sdk-message`（既存維持）: メタイベントのパススルー用に残す

**`execute_agent_query` コマンドの変更**:
- `streaming_message_id: String` パラメータを追加（フロントエンドが `addMessage` で作成したメッセージのID）

**`get_session` コマンドの拡張**:
- `is_streaming: bool` と `streaming_message_id: Option<String>` をレスポンスに追加
- Streaming中はバッファ内容をメッセージにマージして返す

**`respond_agent_permission` コマンドの変更**:
- パーミッション応答時にバッファ内の Permission パートの status を更新し、`streaming-message-updated` を発火

### フロントエンド側の変更

**削除**:
- `useAgentChat.ts`: `globalStreamingMessageIds`, `globalStreamingBuffers`（モジュールスコープMap）
- `useSessionStore.ts`: `flushStreamingBuffer`, `clearFlushState`, `FlushState`, `flushStates`, `FLUSH_INTERVAL_MS`
- `agentChatReducer.ts`: `APPEND_STREAMING`, `APPEND_THINKING`, `APPEND_TOOL_USE`, `APPEND_TOOL_RESULT`, `APPEND_TASK_STATUS`, `ADD_PERMISSION_PART`, `RESOLVE_PERMISSION_PART`
- `useAgentSdkListeners.ts`: `handleStreamingContent`, `handleTaskMessage`, `handleResultErrors` のパースロジック、`flushStreamingBuffer` 呼び出し、アンマウント時フラッシュ

**追加・変更**:
- `agentChatReducer.ts`: `SET_STREAMING_MESSAGE` アクション追加（Rustから受け取ったメッセージでそのまま差し替え）
- `useAgentSdkListeners.ts`:
  - `streaming-message-updated` リスナー → `SET_STREAMING_MESSAGE` をdispatch
  - `agent-streaming-started` リスナー → `START_STREAMING` をdispatch
  - メタイベント（`permissionMode`、`supported_commands`）のハンドリングは維持
- `useAgentChat.ts`:
  - `selectSession` — バッファマージ不要。`getSession` の `is_streaming` に基づき `START_STREAMING` をdispatch
  - `startQuery` — `execute_agent_query` に `streamingMessageId` を渡す
  - `respondPermission` — バッファ操作を削除。コマンド呼び出しのみ（Rust側で `streaming-message-updated` が発火しUIが自動更新される）

### 検討した代替案
- フロントエンドのモジュールスコープMapでバッファを保持する案: コンポーネントライフサイクル回避策を積み重ねる形になり、ストリーミング状態の復元・アンマウント中の完了検知など追加の回避策が必要で複雑化するため却下
- Rust側で蓄積しつつフロントエンドでもパースを維持する案: ロジックが二重になるため却下
- MainLayout レベルでuseAgentChatを管理する案: コンポーネント階層の大幅な変更が必要で影響範囲が広すぎるため却下

### リスク
- Rust側のSDKメッセージパース処理の新規実装: フロントエンドの既存パースロジック（`useAgentSdkListeners.ts` の `extractStreamingDelta`、`handleStreamingContent` 等）をRustに移植する作業量がある
- `streaming-message-updated` のスロットル間隔: 短すぎるとIPC負荷、長すぎると表示遅延。50-100msを起点に調整
- フロントエンドのreducerアクション大幅削除に伴う既存テストの書き直し

### 影響するテスト
- Rustテスト: SDKメッセージのパース → MessageParts蓄積ロジック
- Rustテスト: `get_session` のストリーミングバッファマージ
- Rustテスト: 定期永続化の動作
- フロントエンドテスト: `useAgentChat` — モジュールスコープMap関連テスト削除、`is_streaming` ベースの状態復元テスト追加
- フロントエンドテスト: `useAgentSdkListeners` — パース処理テスト削除、`streaming-message-updated` ハンドリングテスト追加
- フロントエンドテスト: `useSessionStore` — `flushStreamingBuffer` 関連テスト削除
