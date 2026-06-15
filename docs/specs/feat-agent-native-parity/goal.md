# Goal: AgentChat 本文表示UI ネイティブパリティ改善

このドキュメントは **Codex に `/goal` で渡すための引き渡し文書**。
取捨と設計の確定は [triage.md](./triage.md) と [design.md](./design.md) に正本がある。本ドキュメントは実装エージェントが「読み返しを最小化して着手できる」ように要点を集約した抜粋。

矛盾時は [triage.md](./triage.md) と [design.md](./design.md) を優先する。

---

## 関連ファイルパス（プロジェクトルート: `/Volumes/siro33950_SSD_1/workspace/releash`）

### 設計ドキュメント
- 正本（取捨）: `docs/specs/feat-agent-native-parity/triage.md`
- 正本（UI 設計）: `docs/specs/feat-agent-native-parity/design.md`
- 本ファイル: `docs/specs/feat-agent-native-parity/goal.md`

### 主要 Rust ファイル
- `MessagePart` 定義: `src-tauri/src/usecase/agent_session/session/mod.rs:12-104`
- Claude SDK 取り込み: `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - thinking_delta: `:1667, 1795`
  - task_started/progress/notification: `:1887-1924`
  - compact_boundary / hook_*: `:1926-1974`
- Codex app-server 取り込み: `src-tauri/src/infrastructure/agent_session/runtime/codex_app_server.rs`
  - item_tool_name: `:262-269`
  - item_started_message / item_completed_message: `:272-344`
  - command output delta / file_change patch: `:347-358`
  - goal updated / cleared: `codex_goal_updated_message`, `codex_goal_cleared_message`
  - runtime status: `codex_runtime_status_message:382`
- Permission 正規化: `src-tauri/src/adaptor/controller/command/agent_session/permission.rs:182-189`
- Tool 分類: `src-tauri/src/adaptor/controller/command/agent_session/tool_activity.rs:10-46`
- Command palette: `src-tauri/src/adaptor/controller/command/agent_session/command_palette.rs:55-104`
- PermissionMode マッピング: `src-tauri/src/infrastructure/agent_session/runtime/permission_flags.rs`

### 主要フロント ファイル
- 本文描画 switch: `src/components/panels/AgentChatPanel/ChatSessionView.tsx:277-406`
  - ThinkingPart: `:189-222`
  - SystemNotificationItem: `:176-187`
  - isStatusOpen / runtime status panel: `:536, 1436-1504`
- Activity Log（tool 系UI）: `src/components/panels/AgentChatPanel/ActivityLog.tsx`
  - CollapsibleError: `:38-`
  - CommandToolActivity: `:239-273`
  - DefaultToolActivity: `:275-330`
  - TaskToolActivity: `:439-`
- AgentEditPreviewPanel: `src/components/panels/AgentChatPanel/AgentEditPreviewPanel.tsx`
- PermissionDialog: `src/components/panels/AgentChatPanel/PermissionDialog.tsx`
- BoundSessionChat: `src/components/panels/AgentChatPanel/BoundSessionChat.tsx`
- AgentChatPanel: `src/components/panels/AgentChatPanel/AgentChatPanel.tsx:363-394`（runAgentCommand）
- StreamMessage: `src/components/panels/AgentChatPanel/StreamMessage.tsx`
- MessageInput: `src/components/panels/AgentChatPanel/MessageInput.tsx`
- MessagePart 型: `src/types/session.ts:91-139`
- PermissionMode 型: `src/types/session.ts:3-17`

### 共通 Diff コンポーネント（B6 で共通化対象）
- `src/components/panels/DiffViewerSection.tsx`
- `src/components/panels/CodeDiffViewer.tsx`
- `src/components/panels/ShikiDiffViewer.tsx`
- `src/components/panels/MarkdownDiffViewer.tsx`
- `src/components/panels/ImageDiffViewer.tsx`
- `src/components/panels/DiffToolbar.tsx`（Open in editor の参考: `:62`）

### SDK 型定義
- Claude Agent SDK: `node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts`
  - SDKTaskStartedMessage: `:4060` / SDKTaskProgressMessage: `:4038` / SDKTaskNotificationMessage: `:4020` / SDKTaskUpdatedMessage: `:4084`
  - SDKThinkingTokensMessage: `:4106` / SDKToolProgressMessage: `:4115` / SDKToolUseSummaryMessage: `:4126`
  - SDKPermissionDeniedMessage: `:3751`
  - permissionMode: `:1698-1704`
- Codex SDK: `node_modules/@openai/codex-sdk/dist/index.d.ts`
  - ThreadItem: `:104` / ReasoningItem: `:72` / CommandExecutionItem: `:6` / FileChangeItem: `:28` / McpToolCallItem: `:42` / WebSearchItem: `:78` / TodoListItem: `:98` / ErrorItem: `:84`
  - SandboxMode: `:236` / ApprovalMode: `:235`

---

## ゴール総括

Releash の AgentChat 本文表示 UI を Claude/Codex 本家寄せで再設計し、Codex のネイティブイベントを正規化して両 backend で一貫した見え方にする。**取捨選択ベースで進める**（本家全再現ではない）。

横断ルール：
- 「**本家 UI に寄せる**」。ただし Releash で不要な項目は本家にあっても見送る。
- Claude/Codex で同種振る舞いの UI が衝突したら **共通 UI に統一**（正規化維持）。迷ったら Claude UI を baseline とする。

---

## A-1. MessagePart の最終形

#### 現状の 9 variant（`src-tauri/src/usecase/agent_session/session/mod.rs:12-104`）
`Thinking` / `Text` / `ToolUse` / `ToolResult` / `Error` / `Permission` / `TaskStatus` / `SystemNotification` / `Image`

#### 改廃まとめ

| variant | 改廃 | 内容 |
|---|---|---|
| `Thinking` | ✏ 拡張なし、用途追加 | Codex `reasoning` を Rust 側でこの variant に正規化（B3） |
| `Text` | 変更なし | — |
| `ToolUse` | 用途追加 | Codex `web_search` を `tool="WebSearch"` 相当、`command_execution` を `tool="Bash"` 相当、`file_change` を `tool="Edit"` 相当、`mcp_tool_call` を `tool="mcp__<server>__<tool>"` 相当に Rust 側で正規化（B4/B5/B6/B7） |
| `ToolResult` | 変更なし。ただし発生ロジック改修 | output delta を tool_use_id でグルーピングして単一 result に集約（#8） |
| `Error` | 用途追加 | Codex `error` item を Rust 側でこの variant に正規化（B10） |
| `Permission` | 用途追加 | Codex `request_user_input` を `kind="ask_user_question"` の Permission に正規化（#17）。**permission_denied** はここに `status="denied"` で乗せる第一候補（実装時判断）（#13） |
| `TaskStatus` | 振る舞い改修 | `task_updated` を bridge で取り込み、同一 `task_tool_use_id` の TaskStatus を patch（#11） |
| `SystemNotification` | 🗑 縮小 | `notification_type` を **`compaction` のみ**に絞る。`hook` / `files_persisted` / `local_command_output` を削除（B12） |
| `Image` | 変更なし | — |
| **NEW**: `TodoListSnapshot`（仮称） | ➕ 新規追加 | session 内で最新スナップショット 1 件だけ保持。Claude `TodoWrite` と Codex `todo_list` の両方からの更新を集約（B8） |

> 注: `Permission` への `permission_denied` 相乗りで足りない場合（decision_reason 等の情報量が `request` 内に納まらない）、新 variant `PermissionDenied` を切る。実装時に判断。

> 注: フロント側 `src/types/session.ts:91-139` の `MessagePart` 型も Rust と同期して更新する。

---

## A-2. 削除タスク一覧

**Rust 側**
- `bridge_common.rs`: 取り込み停止
  - `hook_started` / `hook_progress` / `hook_response`（→ `SystemNotification(hook)` 経路ごと削除）
  - `files_persisted`（→ `SystemNotification(files_persisted)` 経路ごと削除）
  - `local_command_output`（→ `SystemNotification(local_command_output)` 経路ごと削除）
- `codex_app_server.rs`: 経路ごと削除
  - `codex_goal_updated_message` / `codex_goal_cleared_message`（B14）
  - `codex_runtime_status_message`（B15）
- `MessagePart::SystemNotification` の `notification_type` enum から該当値を削除
- output delta の毎回 tool_result 生成ロジック（`codex_app_server.rs:347-358`）を削除して、tool_use 内 buffer 更新に置換

**フロント側**
- `ChatSessionView.tsx`:
  - 行 536 `isStatusOpen` state、行 1436-1504 のステータスパネル全体
  - `codexRuntimeStatus` prop の受け取り（行 424, 490, 1195-1202）
- `BoundSessionChat.tsx`: `getSessionCodexGoal` 関連の伝搬・呼び出しの完全削除
- 現 `AgentEditPreviewPanel.tsx` の独自簡易 diff 実装（DiffViewerSection 共通化に伴い廃止）

**型 / 設定**
- `src/types/session.ts` の `MessagePart` を Rust と同期させて variant 改廃を反映
- `MessagePart::SystemNotification` の `notification_type` 列挙値も TS 側で同期

---

## A-3. 実装順序・依存関係

> 「先にやらないと別ブロックが書けない」という強い依存だけ示す。並行可能な部分は順不同。

```
Phase 1 — 基盤（並行可、他の前提）
  ├── A-1 MessagePart 改廃（variant 追加 / 縮小）
  ├── A-2 削除タスクの bridge / runtime 側の取り込み停止
  └── B12 System Notification（compaction のみ残す）

Phase 2 — Plan モデル化（#18）★ B11 の前提
  └── PermissionMode と独立した PlanMode (ON/OFF) の追加
        ├── Rust: permission_flags.rs に Plan 経路を追加
        ├── Claude: permissionMode='plan' 送信ロジック
        ├── Codex: Plan mode 起動方法を実装（要 B-1 事前調査）
        └── フロント: MessageInput に Plan トグル追加

Phase 3 — Codex 取り込み正規化
  ├── B3 reasoning → MessagePart::Thinking
  ├── B10 error item → MessagePart::Error
  ├── #17 request_user_input → Permission(kind=ask_user_question)
  └── B5/B6/B7 の tool 正規化（CodexCommand→Bash相当 / CodexFileChange→Edit相当 / CodexMcpTool→mcp__... 相当）
        ※ #8 output delta 集約は B5 と同時に実装

Phase 4 — Claude 取り込み補強
  ├── #11 task_updated → TaskStatus patch
  ├── #13 permission_denied → Permission/Error への乗せ（実装時判断）
  └── B8 TodoListSnapshot variant 追加 + Claude TodoWrite / Codex todo_list 両方の集約

Phase 5 — UI 見直し（本家寄せ）
  ├── B1 / B2 Human/Agent カードレイアウト + msg 単位タイムスタンプ＋コピー
  ├── B3 ThinkingPart 装飾（淡色背景・アイコン・トーンダウン・自動展開折りたたみ）
  ├── B4 read 系 ActivityRow
  ├── B5 CommandToolActivity 作り直し（exit_code / status / terminal 風 / live 出力）
  ├── B6 DiffViewerSection 共通化 + 「Diffで開く」「Open in editor」
  ├── B7 MCP 軽見直し
  ├── B8 TODO List フッター UI + 本文 1 行ログ
  ├── B9 TaskToolActivity 状態別アイコン/色/サマリ
  ├── B10 Error 本家寄せ専用コンポーネント
  ├── B11 Permission UI 見直し（外枠/選択肢枠/背景色削除、回答カード B1 共通化）
  └── 横断 余白/グルーピング調整

Phase 6 — 削除完了
  ├── B14 Codex goal 経路の完全除去
  ├── B15 runtime status パネル完全除去
  └── A-2 削除タスクの残りクリーンアップ
```

**依存の急所**
- Phase 2 が終わらないと B11 Permission UI（pending/answered 振る舞い）が確定できない
- Phase 3 の B5 / B6 / B7 は Rust 側 tool 名正規化が前提。これが無いと Phase 5 の UI 流用が動かない
- Phase 5 の B6 は B6 細部「Diffビューで開く」「Open in editor」の既存 API 名を B-3 で確定してから着手

---

## B-1. Codex Plan mode 起動方法（要事前調査）

Codex SDK の `ApprovalMode` / `SandboxMode` は別軸で、Plan mode はそれらとは別の起動経路を持つ。

**調査タスク**
- `node_modules/@openai/codex-sdk/dist/index.d.ts` の `Thread`/`Codex`/`Input`/`TurnOptions` を確認し、Plan mode 起動の API（メソッド/オプション/環境変数）を特定
- 既存 `src-tauri/src/infrastructure/agent_session/runtime/codex_app_server.rs` の起動経路にどう紐づけるか確認
- Plan ON 時の `sandboxMode` / `approvalPolicy` の固定方針を決める（read-only + on-request 等）

成果物: `src-tauri/src/infrastructure/agent_session/runtime/permission_flags.rs` への Plan 経路実装。

---

## B-2. MCP 正規化マッピング

**Codex `mcp_tool_call` item**:
```
{ id, type: "mcp_tool_call", server, tool, arguments,
  result: { content[], structured_content }?, error: { message }?, status }
```

**Claude `mcp__server__tool` 形式**:
```
ToolUse { tool: "mcp__<server>__<tool>", input: <arguments>, id }
ToolResult { content: <result text>, is_error: <error.is_some()>, tool_use_id: <ToolUse.id> }
```

**変換規則**:
- `tool` 名: `mcp__${server}__${tool}` 形式に組み立てる
- `input`: Codex の `arguments` をそのまま JSON Value として渡す
- `result`: Codex の `result.content[]` を Claude tool_result の content text として連結（複数 content がある場合は改行区切り）
- `error`: Codex `error.message` を `is_error=true` の `ToolResult.content` に格納

---

## B-3. 「Diff ビューで開く」「Open in editor」既存 API

**「Open in editor」**:
- 呼び出し: `invoke("open_in_editor", { filePath })`
- 参照箇所: `src/components/panels/DiffToolbar.tsx:62`、`src/components/panels/ReviewPanel.tsx:550, 623`

**「Diff ビューで開く」**:
- 既存の Diff ビュー（`DiffViewerSection`）のルーティング/開閉 API はワークスペースのトップレベルが管理している。具体 invoke 名や props ハンドラを実装時に特定して合わせる。
- 候補: `WorktreeView.tsx` / メインレイアウト系コンポーネント周辺のグローバルストア操作の特定が必要。
- **未確定**: 実装時に DiffToolbar 経由で開くフローを参考に invoke 名と props 経路を確定。

---

## B-4. B10 tool 内 CollapsibleError 共通化（後追い）

design.md `### B10 Error` 末尾に保留した項目。**実装の余力次第**で B4/B5/B6/B7 内エラー表示も B10 の新スタイルに揃える。今回スコープは B10 本体のみ確定で、tool 内エラーは別作業。

---

## C-1. テスト方針

- **Rust unit**:
  - `bridge_common.rs` の追加/削除する SDK イベントごとの正規化テスト（task_updated patch、permission_denied、Codex reasoning/todo/web_search/error/mcp_tool_call/command_execution/file_change）
  - `codex_app_server.rs` の output delta 集約テスト（複数 delta → 1 result）
  - `permission_flags.rs` の Plan mode 経路テスト（Claude/Codex 両方向）
  - MCP 正規化マッピングテスト
- **TSX component**:
  - 新規/見直しコンポーネント単位（ThinkingPart, CommandToolActivity, FileChangeRow, B11 PermissionDialog, TodoListFooter, B10 Error 専用部品）
  - msg 単位ラッパーと B1/B2 タイムスタンプ＋コピー
- **既存テストの修正方針**:
  - 削除する notification type / Codex goal / runtime status パネル / output delta 経路のテストは削除
  - PermissionDialog の見た目テストは新スタイルに合わせて DOM 期待値更新
- **integration（Playwright）**:
  - turn 全体（Human → reasoning → tool → text → permission → answer）の流れが描画されるシナリオ
  - Plan モードトグルが Claude/Codex 両方の挙動に影響するシナリオ

---

## C-2. scope-out（やらないこと）

- **フッター / MessageInput 全体の UI 見直し** — 別 Spec で扱う。本タスクでは Plan モードトグル追加のみ MessageInput を触る。
- **スキル選択 UI トリガー変更（$ → /）** — commit `cc5b3fa` で完了済み。本タスク範囲外。
- **tool_progress / tool_use_summary / thinking_tokens の取り込み** — triage で見送り確定（#9 / #10 / #12）。bridge で破棄。
- **Codex goal row 復活 / Codex runtime status パネル復活** — triage で完全削除確定（#14 / #15）。
- **runAgentCommand × palette の整合性チェック** — triage #16 で対応不要確定（現状一致）。
- **編集ボタン（B1 Human Message）** — 確定で見送り。コピーボタンのみ。
- **長時間 tool の経過秒・tool まとめ・思考トークン進捗の表示** — 全て見送り。
- **MCP 専用 UI 新設** — 共通 DefaultToolActivity 系（B7）。read/write 分岐も実装しない（1 つの B7 UI に統一）。
- **`permission_denied` の variant 新設** — 第一候補は既存 Permission(status="denied") への乗せ。新 variant は最後の手段（実装時判断）。

---

## C-3. 完了の定義（DoD）

以下が全て満たされたら本タスク完了とする。

1. **正規化**: Codex セッションで以下が個別 MessagePart として本文に描画される
   - reasoning（B3）/ todo_list（B8）/ web_search（B4）/ error（B10）
   - command_execution（B5、output delta は単一 running block で逐次更新）
   - file_change（B6、DiffViewerSection 共通化）
   - mcp_tool_call（B7）
2. **Claude SDK 取り込み**:
   - `task_updated` が同一 task の TaskStatus を patch する
   - `permission_denied` が Permission(denied) または Error として可視化される
   - 未対応イベント（tool_progress / tool_use_summary / thinking_tokens）は警告なく無視される
3. **Plan モード**:
   - MessageInput の PermissionMode セレクタ隣にトグルがある
   - ON 時、Claude には `permissionMode='plan'`、Codex には Plan mode 起動が送信される
   - OFF へ戻すと Claude では元の PermissionMode に復帰する
   - Plan ON 時に Codex `request_user_input` ツール呼び出しが Permission(ask_user_question) として描画される
4. **UI 見直し**（詳細は [design.md](./design.md) の各 B-x セクション参照）:
   - B1×B2 が Claude.ai 風（Human=右寄せ淡色カード+時刻+コピー、Agent=全幅フラット、msg 単位タイムスタンプ+コピー）
   - B3 ThinkingPart が新装飾（淡色背景+アイコン+トーンダウン+自動展開/折りたたみ+Markdown）
   - B5 CommandToolActivity が新装飾（terminal 風+exit_code/status+live 出力）
   - B6 が DiffViewerSection 完全共通化+「Diffビューで開く」+「Open in editor」
   - B11 PermissionDialog が外枠/選択肢枠/背景色なしのフラット+回答カードが B1 と同等
   - 上記すべてで Codex セッションでも同等の見た目が出る
5. **削除**:
   - Codex goal row / runtime status パネルの跡形が UI / Rust から消えている
   - B12 が compaction のみで他 notification type が来ても無視される
6. **品質**:
   - `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る
   - `pnpm lint` / `pnpm test` / `pnpm build` が通る
   - Playwright integration テストの主要シナリオが通る
