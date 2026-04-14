## 要求

**種別**: バグ修正
**現在の挙動**: Claude CodeでCompaction（会話圧縮）が発生しても、ReleashのAgentChat上に通知や表示がなく、ユーザーが気づけない
**期待する挙動**: Compaction発生時にAgentChat上で表示し、ユーザーがAgentの作業状況を把握できるようにする
**再現手順**:
1. ReleashのAgentChatからClaude Codeセッションを実行する
2. コンテキストウィンドウが上限に達しCompactionが発生する
3. AgentChat上にCompactionの発生を示す通知や表示がない
**背景**: Compactionが発生するとAgentのコンテキストが圧縮され、以降の動作に影響する可能性があるが、現状ではReleash上でその発生を確認する手段がなく、Agentの作業状況の把握が困難になる

## 振る舞い定義

```gherkin
Feature: SDKシステムメッセージ通知
  AgentChatセッション中にClaude Code SDKからシステムメッセージを受信した場合、
  ユーザーがAgentの作業状況を把握できるようAgentChat上にシステムメッセージを表示する

  Background:
    Given AgentChatセッションが実行中である

  Rule: Compactionの開始と完了を1つのシステムメッセージとして表示する
    Scenario: Compaction開始
      When SDKがstatus=compactingメッセージを送信する
      Then AgentChatにCompaction進行中のシステムメッセージが追加される

    Scenario: Compaction完了
      Given AgentChatにCompaction進行中のシステムメッセージがある
      When SDKがcompact_boundaryメッセージを送信する
      Then そのシステムメッセージがCompaction完了の表示に更新される

  Rule: Hookの開始と完了を1つのシステムメッセージとして表示する
    Scenario: Hook開始
      When SDKがhook_startedメッセージを送信する
      Then AgentChatにHook名とイベント名を含む進行中のシステムメッセージが追加される

    Scenario: Hook完了
      Given AgentChatにHook進行中のシステムメッセージがある
      When SDKが同じhook_idのhook_responseメッセージを送信する
      Then そのシステムメッセージがHook名と実行結果を含む完了表示に更新される

  Rule: ファイル永続化をシステムメッセージとして表示する
    Scenario: ファイル永続化の通知
      When SDKがfiles_persistedメッセージを送信する
      Then AgentChatに永続化されたファイル名を含むシステムメッセージが表示される

  Rule: ローカルコマンド出力をシステムメッセージとして表示する
    Scenario: ローカルコマンド出力の通知
      When SDKがlocal_command_outputメッセージを送信する
      Then AgentChatにコマンド出力内容のシステムメッセージが表示される

  Rule: initとtask関連とpermissionMode同期は表示しない
    Scenario: セッション初期化メッセージは表示しない
      When SDKがinitメッセージを送信する
      Then AgentChatにシステムメッセージは追加されない

    Scenario: タスク関連メッセージは既存のTaskStatusとして処理する
      When SDKがtask_started/task_progress/task_notificationメッセージを送信する
      Then 既存のTaskStatusパートとして処理される

    Scenario: permissionMode同期は表示しない
      When SDKがstatus=nullかつpermissionMode付きのstatusメッセージを送信する
      Then AgentChatにシステムメッセージは追加されない
```

## 実装仕様

**対応方針**: 振る舞い定義のSDKシステムメッセージ通知を実現するために、Rust側の `accumulate_sdk_message()` に新しいシステムsubtype（compacting/compact_boundary/hook_started/hook_response/files_persisted/local_command_output）の処理を追加し、新しいMessagePartバリアント `SystemNotification` で蓄積・永続化する。フロントエンドは受け取ったpartをインライン表示するのみ。

**対象コンポーネント**:
- `src-tauri/src/session/mod.rs`: `MessagePart` enumに `SystemNotification` バリアント追加
- `src-tauri/src/agent_sdk.rs`: `accumulate_sdk_message()` にsubtype分岐を追加。Compaction/Hookは同一partを更新するパターン
- `src/types/session.ts`: TypeScript側 `MessagePart` に `system_notification` を追加
- `src/components/panels/AgentChatPanel/AgentChatPanel.tsx`: `AgentMessageParts` 内で `system_notification` partをインライン表示

**SystemNotificationのフィールド設計**:

Rust:
```rust
SystemNotification {
    notification_type: String,  // "compaction" | "hook" | "files_persisted" | "local_command_output"
    status: String,             // "in_progress" | "completed" | "error"
    label: String,              // 表示用テキスト
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,     // 追加情報
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_id: Option<String>,    // Hook開始→完了の紐付け用
}
```

TypeScript:
```typescript
| {
    type: "system_notification";
    notificationType: "compaction" | "hook" | "files_persisted" | "local_command_output";
    status: "in_progress" | "completed" | "error";
    label: string;
    detail?: string;
    hookId?: string;
  }
```

**各SDKメッセージの変換ロジック**:

| SDKメッセージ | notificationType | 初期status | label | detail |
|---|---|---|---|---|
| `status=compacting` | compaction | in_progress | "Compacting conversation..." | None |
| `compact_boundary` | compaction | completed | "Conversation compacted" | `"trigger={trigger}, {pre_tokens} tokens"` |
| `hook_started` | hook | in_progress | `"{hook_name} ({hook_event})"` | None |
| `hook_response` | hook | completed/error | `"{hook_name} ({hook_event})"` | `"outcome={outcome}, exit_code={exit_code}"` |
| `files_persisted` | files_persisted | completed | "Files persisted" | ファイル名のカンマ区切りリスト |
| `local_command_output` | local_command_output | completed | "Command output" | content（200文字でtruncate） |

**開始→完了の同一part更新メカニズム**:

Compaction:
- `status=compacting` 受信時: `SystemNotification { notification_type: "compaction", status: "in_progress", ... }` を `streaming_parts` にpush
- `compact_boundary` 受信時: `streaming_parts` を逆走査し、`notification_type == "compaction" && status == "in_progress"` に一致する最新のpartを `status: "completed"` に更新。deltaとして更新後のpartを送信。該当partがなければcompleted状態で新規push

Hook:
- `hook_started` 受信時: `hook_id` 付きの `SystemNotification` を `streaming_parts` にpush（`hook_id` が空文字または未設定の場合は `None` として保存）
- `hook_response` 受信時: `streaming_parts` を逆走査し、`notification_type == "hook" && status == "in_progress" && hook_id` が一致するpartを検索してstatus/detailを更新。deltaとして更新後のpartを送信。該当partがなければ新規push

**非表示メッセージ（既存処理を維持）**:
- `init`: `accumulate_sdk_message` で `false` を返す（既存）
- `task_started/task_progress/task_notification`: 既存の `TaskStatus` 処理を維持（既存）
- `status` で `permissionMode` 付き: フロントエンドでpermissionMode同期として処理（既存の `agent-sdk-message` フォワード）

**検討した代替案**:
- フロントエンド側のみで処理: Rust変更なしだが永続化されず、セッション再読み込み時に消失。全ロジックをRustに寄せる方針に反する

**OSSプロジェクト調査結果**:
- VS Code/Genie/claude-view/OpenCow/GSD/BotVa/ACP の7プロジェクトを調査
- 全プロジェクトが `type` → `subtype` の2段階分岐を使用
- `compact_boundary` はほぼ全プロジェクトで処理対象
- `files_persisted`/`local_command_output` は多くのプロジェクトで未実装またはno-op
- Hook系は対応が分かれる（トレーシング/監査ログ/スキップ）

**影響するテスト**:
- Rust単体テスト: `session/mod.rs` — `SystemNotification` のserde roundtrip、後方互換性
- Rust単体テスト: `agent_sdk.rs` — `accumulate_sdk_message()` の各subtypeの変換テスト（compacting, compact_boundary, hook_started, hook_response, files_persisted, local_command_output）
- Rust単体テスト: `agent_sdk.rs` — Compaction/Hookの開始→完了で同一partが更新されるテスト
- フロントエンドテスト: `AgentChatPanel` — `system_notification` partの表示テスト
- フロントエンドテスト: `session.ts` — `system_notification` が無い古いJSONのデシリアライズ後方互換性
