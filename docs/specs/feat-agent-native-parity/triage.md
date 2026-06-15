# AgentChat 本文表示UI 改善 — 取捨選択トラッキング

関連: ブランチ `feat/agent-native-parity` / [design.md](./design.md) / [goal.md](./goal.md)（Codex 引き渡し用）

## Context（なぜ）

Releash の AgentChat 本文 UI は Claude/Codex のイベントを共通 `MessagePart`（9 variant）に正規化して最低限表示できているが、本家アプリ（Claude Code / Codex CLI）が持つ「実行中アイテム」「reasoning/todo/web search」「progress 系」「Codex native item」の多くが欠落または粗く畳み込まれている。特に Codex は native ThreadItem モデルをかなり失っている。

本タスクのゴールは **ネイティブパリティ（全再現）ではなく、Releash のユースケースに照らした取捨選択**。

## 確定事項（合意済み）

- **ゴール**: Releash として取捨選択（本家差分は参考情報。必要な表示だけ個別合意で入れる）
- **進め方**: 差分マップの項目ごとに一つずつ「入れる/見送る」を合意し、本ドキュメントに都度記録
- **横断テーマ**: 「UI 自体の見直し」をしたい意向あり（範囲は審議中。下記参照）

## 横断テーマ: UI 自体の見直し = ✅「本家UIに寄せる」で確定

見直しの軸は **Claude Code / Codex CLI 本家の見た目・振る舞いに寄せて再現する** こと。
- 「入れる」と決めた項目は、本家 UI の見せ方（レイアウト/アイコン/折りたたみ/実行中表示）に寄せて実装する。
- 取捨選択のゴールは維持。Releash で不要な項目は本家にあっても見送る。本家にある＝必ず入れる、ではない。

### Claude 本家 / Codex 本家 で UI が衝突する場合のルール（確定）
- **共通UIに統一する**（正規化を維持）。同種の振る舞いは1つの Releash 標準表示に寄せ、backend が変わっても見た目を変えない。現アーキテクチャ（共通 `MessagePart`）と整合。
- **統一時の見せ方で迷う場合は Claude の UI を基本（baseline）とする。**

---

## 取捨選択 対象一覧

凡例 — 対応: ✅入れる / ⛔見送る / ⬜未定　|　方針: 実装/表示の合意メモ

### A. Codex: 現状 infra が item を拾ってすらいない（本家にあるが完全欠落）

| # | 項目 | 現状 | 本家 | 対応 | 方針メモ |
|---|------|------|------|------|----------|
| 1 | `reasoning`（思考テキスト） | infra が拾わず本文に一切出ない | Codex native ReasoningItem。Claude は thinking part あり | ✅入れる | **現 ThinkingPart を見直してから流用**。ThinkingPart の見た目を本家寄せで作り直したうえで、Codex reasoning も同じ UI で出す。Claude と Codex で表示は共通。 |
| 2 | `todo_list`（計画/進捗チェックリスト） | 無視 | TodoListItem（items[{text, completed}]） | ✅入れる | **Claude TodoWrite と共通UI**。専用 MessagePart variant を新設し、Claude TodoWrite と Codex todo_list を同一のチェックリストUIに集約。両 backend で計画と進捗が見える。 |
| 3 | `web_search`（検索クエリ） | 無視 | WebSearchItem（query） | ✅入れる | **Claude WebSearch と共通UI**。Codex web_search を Claude WebSearch と同じ read 系 tool UI に正規化。query を表示。 |
| 4 | `error`（ErrorItem first-class） | SDK境界エラーのみ。native ErrorItem は無視 | ErrorItem（message） | ✅入れる | **既存 MessagePart::Error に正規化**。codex_app_server で ErrorItem → Error part の変換を追加。フロントは既存 CollapsibleError UI を流用。 |

### B. Codex: tool_use/tool_result に潰している（分類改善で本家相当に）

| # | 項目 | 現状 | 本家 | 対応 | 方針メモ |
|---|------|------|------|------|----------|
| 5 | `command_execution` の command/terminal UI | 汎用 tool_result ✓（Bash 扱いされない） | CommandExecutionItem（command, aggregated_output, exit_code, status） | ✅入れる | **現 CommandToolActivity を見直してから流用**。CommandToolActivity を本家寄せで作り直したうえで、CodexCommand を command 分類に追加して同UIに乗せる。Claude Bash と Codex command_execution を共通UIで表示。 |
| 6 | `file_change` の diff/file-change UI | write/edit preview 対象外 | FileChangeItem（changes[{path, kind}], status） | ✅入れる | **DefaultToolActivity の edit preview を見直してから流用**。AgentEditPreviewPanel を本家寄せで見直し、CodexFileChange を Claude Edit/Write と同じ diff/preview UI に乗せる。両 backend でファイル変更が共通UI。 |
| 7 | `mcp_tool_call` の MCP tool UI | 汎用扱い | McpToolCallItem（server, tool, arguments, result, error, status） | ✅入れる（共通） | **Claude の MCP に合わせ DefaultToolActivity に乗せる**。MCP 専用UIは新設しない（Claude も現状 DefaultToolActivity）。CodexMcpTool を Claude `mcp__...` ツールと整合する tool name に正規化して同UI。 |
| 8 | `output delta` を単一 running block に集約 | delta ごとに tool_result 化 → standalone ✓ が乱立（構造的バグ） | item.updated で同一 item を逐次更新 | ✅入れる（構造修正） | outputDelta を tool_use id に紐づけ、output buffer を逐次更新する形に修正。途中 delta の独立表示を解消し、#5 の CommandToolActivity と live 連動。 |

### C. Claude: bridge_common で落ちている progress 系

| # | 項目 | 現状 | 本家 | 対応 | 方針メモ |
|---|------|------|------|------|----------|
| 9 | `tool_progress`（経過秒） | 未処理（落ち） | SDKToolProgressMessage（elapsed_time_seconds） | ⛔見送る | 経過秒の表示はしない。実行中インジケータのみで状態を伝える。 |
| 10 | `tool_use_summary`（tool まとめ） | 未処理（落ち） | SDKToolUseSummaryMessage（summary, preceding_tool_use_ids） | ⛔見送る | tool まとめは表示しない。tool は個別に見せたまま。 |
| 11 | `task_updated`（task 状態更新） | 未処理（落ち） | SDKTaskUpdatedMessage（patch: status/error/is_backgrounded…） | ✅入れる | bridge で取り込み、同一 task_tool_use_id の TaskStatus を patch（in-place 更新）。task の running/completed/failed/backgrounded を反映。 |
| 12 | `thinking_tokens`（思考トークン進捗） | 未処理（落ち） | SDKThinkingTokensMessage（estimated_tokens） | ⛔見送る | トークン進捗は表示しない。思考本文の表示のみ。 |
| 13 | `permission_denied`（拒否理由の専用表示） | tool_result/error に寄る | SDKPermissionDeniedMessage（decision_reason, message） | ✅入れる | bridge で取り込み、decision_reason と message を保持して表示。新 variant 追加か既存 Permission(status="denied") / Error への正規化のいずれか（実装時に決定）。 |

### D. utility / runtime / palette 整合

| # | 項目 | 現状 | 対応 | 方針メモ |
|---|------|------|------|----------|
| 14 | Codex `goal` row | BoundSessionChat が getSessionCodexGoal を渡さず、Goal row 削除済み | ⛔見送る（完全削除） | 復活させない。infra/listener 側の goal 入り口も含めて整理し、Claude との共通UI方針と整合させる。 |
| 15 | Codex `runtime status` を開くUI | データはあるが isStatusOpen を true にする手段がなく開けない（実質バグ。閉じる X のみ） | ⛔見送る（パネルごと削除） | isStatusOpen、パネル本体、codexRuntimeStatus prop の伝搬まで含めて削除。Claude との共通UI方針と整合。 |
| 16 | `runAgentCommand` × command palette 整合 | runAgentCommand は8コマンドのみ処理。palette が削除済み action を返すと表示されても無反応 | ⛔見送る（対応不要） | 実コード確認の結果、palette（`command_palette.rs:55-104` の AGENT_SHORTCUTS）と runAgentCommand の id 集合は8項目で一致。報告時点の不整合は既に整理済みで現状は問題なし。 |
| 17 | Codex の質問ツール対応（**新規論点**） | Claude `AskUserQuestion` は `permission.rs:188` で `kind="ask_user_question"` として `PermissionDialog` の専用UIに出る。Codex 側は infra に質問取り扱いの実装無し。Codex の質問機能は `request_user_input` ツール経由で **Plan mode 専用** | ✅入れる | `request_user_input` 呼び出しを Rust 側で `kind="ask_user_question"` の Permission に正規化し、Claude `AskUserQuestion` と同じ `PermissionDialog` で受ける。**ただし #18 Plan モード対応が前提**（Plan モード下でしか呼ばれない） |
| 18 | Plan モードの Releash モデル化（**新規論点**） | Claude SDK は `permissionMode='plan'`（4値排他）、Codex SDK は Plan mode を独立軸として持つ。Releash の抽象 `PermissionMode = Ask\|Edit\|Full` は Plan を持たず、両 backend の Plan モードを呼び出せる経路が無い | ✅入れる | **PermissionMode とは独立トグル**として `PlanMode (ON/OFF)` を追加。Plan OFF: 通常の PermissionMode を両 backend に送信。Plan ON: Claude には `permissionMode='plan'` を送信、Codex には Plan mode を起動。**Plan OFF へ戻したとき Claude では PermissionMode 側で選択している権限に復帰**する（PermissionMode 値は Plan 中も内部保持）。Codex の Plan mode 起動方法は未調査・実装時に確認 |

---

## 検証で判明した実装上の事実（参照）

- `MessagePart` は **Rust（`src-tauri/src/usecase/agent_session/session/mod.rs:12-105`）とフロント（`src/types/session.ts:91-139`）の両方に 9 variant**。新 part 追加は両側 + 変換層の改修が必要（rust-first 原則: ロジックは Rust）。
- Codex item → tool 変換: `src-tauri/src/infrastructure/agent_session/runtime/codex_app_server.rs:262-358`（item_tool_name / item_started_message / item_completed_message / output delta）。
- Codex tool 分類: `src-tauri/src/adaptor/controller/command/agent_session/tool_activity.rs:10-46`（Codex 専用名の terminal/diff 扱いなし）。
- Claude SDK 取り込み: `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs:1887-1974`（task_started/progress/notification, compact_boundary, hook_*）。task_updated/tool_progress/tool_use_summary/permission_denied/thinking_tokens は未処理。
- 本文描画 switch: `src/components/panels/AgentChatPanel/ChatSessionView.tsx:277-406`（thinking/text/error/tool_use/tool_result/permission/system_notification/image）。
- runtime status パネル: `ChatSessionView.tsx:536, 1436-1504`（開くボタン無し）。
- runAgentCommand: `src/components/panels/AgentChatPanel/AgentChatPanel.tsx:363-394`（8 コマンド）。palette 取得: 同 :217 `present_agent_command_palette`。

## 進捗ログ

- 確定: 横断テーマ UI見直し = 「本家UIに寄せる」
- 確定: 衝突時ルール = 共通UIに統一（正規化維持）／迷ったら Claude UI を baseline
- 確定: #1 reasoning = ✅入れる、UI は現 ThinkingPart を見直してから流用
- 確定: #2 todo_list = ✅入れる、Claude TodoWrite と共通UIに集約
- 確定: #3 web_search = ✅入れる、Claude WebSearch と共通UI（read系）
- 確定: #4 error = ✅入れる、既存 Error part に正規化
- 確定: #5 command_execution = ✅入れる、CommandToolActivity を見直してから流用
- 確定: #6 file_change = ✅入れる、edit preview を見直してから流用
- 確定: #7 mcp_tool_call = ✅入れる、Claude に合わせ DefaultToolActivity に乗せる
- 確定: #8 output delta = ✅集約（構造修正）
- 確定: #9 tool_progress = ⛔見送る
- 確定: #10 tool_use_summary = ⛔見送る
- 確定: #11 task_updated = ✅入れる（TaskStatus を patch）
- 確定: #12 thinking_tokens = ⛔見送る
- 確定: #13 permission_denied = ✅入れる（variant か既存への正規化かは実装時決定）
- 確定: #14 Codex goal row = ⛔見送る（完全削除）
- 確定: #15 runtime status = ⛔見送る（パネルごと削除）
- 確定: #16 palette 整合 = ⛔見送る（実コードで既に整合済み）
- 確定: #17 Codex 質問ツール = ✅入れる（#18 が前提）
- 確定: #18 Plan モード = ✅入れる（PermissionMode と独立、Plan 解除時に元の権限へ復帰）
- **設計フェーズ完了**: design.md に B1〜B12 + 横断 + Plan モデル化 を全項目確定済み
- 次アクション: 次フェーズ（requirements 化 / behavior 化 / 実装計画）に進む方針を確認

---

## サマリー（16項目の最終配分）

### ✅ 入れる（9 件）

- **#1 reasoning** — 現 ThinkingPart を見直してから流用。Codex reasoning も同UI
- **#2 todo_list** — 専用 MessagePart 新設、Claude TodoWrite と共通チェックリストUIに集約
- **#3 web_search** — Codex web_search を Claude WebSearch と同じ read 系 tool UI に正規化
- **#4 error** — Codex ErrorItem を既存 MessagePart::Error に正規化、CollapsibleError 流用
- **#5 command_execution** — 現 CommandToolActivity を見直してから流用、CodexCommand を command 分類追加
- **#6 file_change** — edit preview を見直してから流用、CodexFileChange を Claude Edit/Write と同UI
- **#7 mcp_tool_call** — Claude に合わせ DefaultToolActivity に乗せる（MCP 専用UIは新設しない）
- **#8 output delta** — outputDelta を tool_use id に紐づけ、output buffer 逐次更新（構造修正）
- **#11 task_updated** — bridge 取り込み、同一 task_tool_use_id の TaskStatus を patch
- **#13 permission_denied** — bridge 取り込み、表示先（新 variant / Permission denied / Error）は実装時決定

### ⛔ 見送る / 削除（7 件）

- **#9 tool_progress** — 経過秒表示はしない
- **#10 tool_use_summary** — tool まとめ表示はしない
- **#12 thinking_tokens** — 思考トークン進捗表示はしない
- **#14 Codex goal row** — 完全削除（infra/listener 入り口も整理）
- **#15 Codex runtime status** — パネルごと削除（isStatusOpen / codexRuntimeStatus prop 含む）
- **#16 palette 整合** — 対応不要（現コードで既に整合済み）

### UI 見直しの帰結（横断）

「入れる」項目のうち **#1 / #5 / #6** は単に「Codex を Claude 側UIに乗せる」のではなく、**Claude 側UI（ThinkingPart / CommandToolActivity / edit preview）自体を本家寄せで見直してから両 backend を乗せる**という方針。これがブランチ feat/agent-native-parity の本丸。
