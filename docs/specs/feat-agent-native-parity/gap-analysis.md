# 実装ギャップ分析 — feat/ui 未コミット差分 vs design.md / goal.md

対象: `feat/ui` ブランチの未コミット差分（2895 insertions / 1390 deletions, 41 files）
基準: [design.md](./design.md)（UI 設計正本）/ [goal.md](./goal.md)（実装引き渡し・DoD）
作成日: 2026-06-15

---

## サマリ

設計の大半は実装済み。Rust 側正規化・削除タスク・Plan モード・Permission UI 見直しはほぼ DoD を満たす。
**残る不足は主にフロント UI の細部 3〜4 点**で、加えて**ユーザー報告の UI 体感問題 4 点**がある。後者は「設計どおりに作った結果かえって使いづらくなった（行き過ぎ）」「設計が詰め切れていなかった」箇所が含まれる。

---

## 1. 実装済み（DoD 充足を確認）

| 領域 | 内容 | 根拠 |
|---|---|---|
| MessagePart 改廃 | `TodoListSnapshot` 追加 / `SystemNotification` を compaction のみに縮小 | `usecase/.../session/mod.rs:94`, `types/session.ts:117,109` |
| Codex 正規化 | reasoning→Thinking / web_search→ToolUse(WebSearch) / command_execution→ToolUse(Bash) / file_change→ToolUse(Edit) / mcp_tool_call→ToolUse(mcp__) / error→Error | `codex_app_server.rs:419-558` |
| output delta 集約 | 同一 tool_use_id へ単一 result を更新 | `bridge_common.rs:1839-1886` |
| Claude SDK | task_updated→TaskStatus patch / permission_denied→Permission(denied) / 未対応イベント(tool_progress 等)は黙殺 | `bridge_common.rs:2086-2247` |
| request_user_input | Codex→Permission(ask_user_question) 正規化 | `codex_app_server.rs:335-355,916-928` |
| Plan モード | PlanMode トグル / Claude `permissionMode='plan'` / Codex `apply_plan_mode()` / OFF 復帰 | `MessageInput.tsx:788`, `bridge_common.rs:1668-1680`, `codex_app_server.rs:67-76` |
| 削除タスク | hook_*/files_persisted/local_command_output 取り込み停止 / Codex goal・runtime status 経路完全除去 | `bridge_common.rs:2298-2303`, `codex_app_server.rs:644-674`, `ChatSessionView.tsx`(isStatusOpen 削除) |
| UI 見直し | B2 msg 単位タイムスタンプ+コピー / B3 Thinking 装飾 / B4 Read 系専用行 / B5 terminal 風 / B6 DiffViewerSection 共通化 / B7 MCP / B9 状態別 / B10 専用 Error / B11 Permission フラット化 | `ChatSessionView.tsx`, `ActivityLog.tsx`, `AgentEditPreviewPanel.tsx`, `PermissionDialog.tsx` |

---

## 2. 不足点（spec 未充足）

### 2-1. B1 Human Message の長文コラプス — 未実装
- 設計: design.md L71「Human 独自の閾値（叩き台 3000 chars / 50 lines）で折りたたむ」
- 現状: `StreamMessage.tsx` の HumanMessageContent に collapse 機構なし。長文入力で UI が縦に伸びる。

### 2-2. B3 Thinking の「完了後 自動折りたたみ」 — 未実装
- 設計: design.md L83「ストリーミング中は自動展開、完了後は自動で折りたたむ」
- 現状: `ChatSessionView.tsx:262` で `useState(true)` 固定。手動トグルのみで、完了後に自動で閉じる遷移がない。

### 2-3. B8 TODO の「本文 1 行ログ」 — 未実装
- 設計: design.md L138 / goal.md C-3「TODO 更新ごとに『TODO を更新しました（3/7 完了）』の 1 行ログを本文に残す」
- 現状: フッター（`TodoListFooter` `ChatSessionView.tsx:182-251`）のみ実装。本文側の更新ログがないため「いつ計画が変わったか」を文脈から追えない。
- 補足: `todo_list_snapshot` part は本文レンダリングで `return null`（L478-479）= 本文に何も出さない設計になっている。

### 2-4. B6 File Change の kind インジケータ（A/M/D） — 部分実装
- 設計: design.md L116「kind インジケータ（add/update/delete）: ファイル名横に A/M/D アイコン」
- 現状: `AgentEditPreviewPanel.tsx:156` で `FileDiff` 統一アイコンのみ。add/update/delete の区別表示なし。
- （+N/-M バッジ・Diff で開く・Open in editor は実装済み）

### 2-5. SystemNotification の型縮小が Rust 側で型表現されていない — 軽微
- Rust の `notification_type` は `String` のまま（enum 化されていない）。実装上は compaction 以外を黙殺するが、型での保証はなし。TS 側は `"compaction"` リテラルに縮小済み。

---

## 3. 実装上の懸念（ユーザー報告の体感問題）

> いずれも feat/ui の実際の挙動として確認済み。設計との関係を併記する。

### 3-1. ユーザメッセージの背景にうっすら色がついていて気持ち悪い
- 箇所: `StreamMessage.tsx:102-103` `rounded-lg border border-primary/15 bg-primary/10`
- 関係: 設計 B1 は「淡い背景色のカード（右寄せ）」を**意図**している（design.md L64）。意図どおりだが、`bg-primary/10`（primary 系の色）の色味が体感的に不快との指摘。
- **対応（確定）**: bg-muted 系に変更。具体値は当 UI のミュートカード慣習（`bg-muted/40` + `border-border`。例: queued turn `ChatSessionView.tsx:1280`、status box `:1487`）に合わせる。`bg-muted` 単色は使わない。
  - `StreamMessage.tsx:177`（HumanMessageContent カード）: `border border-primary/15 bg-primary/10` → `border border-border bg-muted/40`
  - **整合**: B11 回答カード（`PermissionDialog.tsx:187`）は design B11 で「B1 Human Message と同じスタイル」と規定。同じ `border border-border bg-muted/40` に揃える（人間発話の見え方を一貫させる）。
  - mention バッジ内の `bg-primary/10`（`StreamMessage.tsx:187`）はアクセントとして別物のため変更しない。

### 3-2. Permission UI が背景色も枠もなく境界が全く判別できない
- 箇所: `PermissionDialog.tsx:168-173` `PermissionShell`（`mx-3 my-2 overflow-hidden text-xs`、border/bg なし）
- 関係: 設計 B11 が**意図的に**外枠（`border + bg-muted/50`）・選択肢枠・回答済み背景色を削除した結果（design.md L185-188）。**設計の行き過ぎ**。「うるさすぎる」反動でフラットにし過ぎ、ブロックとしての境界が消失。
- **対応（確定）**: PermissionUI 自体に背景色と各選択肢に背景色をつける

### 3-3. AgentMessage の時刻が「返信開始時点」で固定されていて意味不明
- 箇所: `ChatSessionView.tsx:290` `formatMessageTime(msg.timestamp)`
- 原因: `msg.timestamp` はメッセージ生成時（=返信開始時）に確定する値。長時間の生成では「いつ返信が来たか」を表さない。
- 関係: 設計 B2「agent message 単位で 1 セット」（design.md L76）はタイムスタンプの**意味（開始/完了）を規定していない**。
- **対応（確定）**: 完了時刻を完了時にのみつける
- **実装上の前提（検証済み）**: `ChatMessage` には開始時の `timestamp` しか無い（`types/session.ts:155-161`）。完了時刻を出すには backend から完了タイムスタンプを供給する（新フィールド追加 or 完了時に `timestamp` を更新）必要がある。フロントだけでは「完了時刻」を持てない。

### 3-4. AgentMessage の時刻が（シマーの）上にあって意味不明
- 箇所: `ChatSessionView.tsx:1258-1267` — メッセージ単位で `AgentMessageParts` → `AgentMessageMeta`（時刻）の順にレンダリングし、それを `session.messages` 分だけ縦に並べる。
- 原因: メタ行（`AgentMessageMeta`、時刻＋コピー）が各メッセージの**末尾**に付く。`msg.timestamp`（開始時刻）が、生成中はリスト末尾に出ているシマーの**上**に居座って見える。
- 「**Thinking のプログレスバー**」の実体（検証済み）: `ShimmerPlaceholder`（`ShimmerPlaceholder.tsx`、`agent-shimmer` のバー）。`ChatSessionView.tsx:1186` で**全メッセージの後ろの追加仮想行**として描画され、`shimmerLineCount > 0`（`:754-770`、`isStreaming` 中のみ）で表示。turn 完了でこの行は消える。
- **対応（確定）**: 完了時刻を、この Thinking プログレスバー（シマー）の**下**＝ turn 末尾に置く。
  - 生成中はシマーを表示し時刻は出さない（→ 3-3 の「完了時にのみ」と一致）。
  - 完了でシマーが消えたら、その位置（turn 末尾）に完了時刻を 1 つ出す。
  - 現状の per-message `AgentMessageMeta`（各メッセージ末尾、`:1267`）の配置をやめ、turn 末尾（シマー跡）に移す。

---

## 3-5. ツール使用時 UI（B4/B5/B7）— コンポーネント未共通化 / Bash 逸脱 / ラベル整形不統一

design.md の横断ルール「共通 UI に統一」（design.md L9, B4/B5/B7）に対し、実装は**見た目の共通化はあるが、コンポーネント自体が共通化されていない**。

### 3-5-1. ツール行が 5 つの重複コンポーネントに分裂
- `ActivityLog.tsx` に `ReadToolActivity`(L378) / `CommandToolActivity`(L409) / `WriteToolActivity`(L461) / `McpToolActivity`(L514) / `DefaultToolActivity`(L554) が並存。
- 共通ヘッダー `ToolActivityHeader`(L330) は共有しているが、各コンポーネントは「state・展開ボディ・結果表示」をそれぞれ重複実装。category 追加・挙動変更のたびに 5 箇所を直す必要があり、差異が生まれやすい（実際 Bash が逸脱）。
- **あるべき姿**: ツール行は単一コンポーネントに共通化し、category 差分（先頭アイコン / 展開ボディの種類）だけをパラメータ化する。

### 3-5-2. Bash（CommandToolActivity）だけ行構造が逸脱
- ラベルが `font-mono text-foreground/85`（他行は muted・通常フォント）→ 一段明るく等幅で浮く（`ActivityLog.tsx:432`）
- ヘッダーの**外**に追加の `SmallCopyButton` を置き、行のレイアウトが他と異なる（`:445`）
- `state` + `exit NN` バッジを meta 表示（`:433-442`）、展開時の出力が terminal 暗背景（`:454`）
- 他ツール行は「chevron/spinner + アイコン + ラベル + 右端ステータスアイコン」のみ。Bash だけ要素構成が違う。

### 3-5-3. 表示ラベルのソースが不統一
- Read: `presentation.label`（"Explored ..."）
- Command: `presentation.label`（コマンド文字列）
- MCP: `mcpLabel(entry.tool)` を**フロントで再整形**（`ActivityLog.tsx:90-94`）。Rust 側 `tool_activity.rs:124` は MCP を `tool_name` のまま返す
- Write/Default: フロントで `${entry.tool} ${summary}` を組み立て（`ActivityLog.tsx:474,573`）
- → 同じ「1 行サマリ」なのにソースと整形場所がバラバラ。rust-first 原則（整形は Rust）にも反し、design B4「JSON を出さず 1 行整形」の責務配置が崩れている。
- **あるべき姿**: 表示ラベルは Rust `present_agent_tool_activity` の単一フィールドに集約し、フロントは category 問わずそれを表示するだけにする。

