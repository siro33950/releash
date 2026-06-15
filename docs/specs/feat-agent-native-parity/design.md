# AgentChat 本文表示UI 改善 — 設計（ありたい姿）

関連: [triage.md](./triage.md) / ブランチ `feat/agent-native-parity`

## 前提（triage で確定済み）

- **ゴール**: Releash として取捨選択。本家差分は参考。
- **横断テーマ**: 「本家UIに寄せる」。
- **衝突時ルール**: 共通UIに統一（正規化維持）。迷ったら Claude UI を baseline。
- **✅入れる項目**: #1 reasoning, #2 todo_list, #3 web_search, #4 error, #5 command_execution, #6 file_change, #7 mcp_tool_call, #8 output delta, #11 task_updated, #13 permission_denied
- **⛔見送る項目**: #9 tool_progress, #10 tool_use_summary, #12 thinking_tokens, #14 Codex goal, #15 Codex runtime status, #16 palette 整合

## 設計の進め方

「ありたい姿を先に描く」に基づき、以下の順で固める：

1. **本文に出すべき UI ブロックの理想セット**（下記の一覧）を合意する
2. 各ブロックの「役割／見た目／何の機能を集約するか／本家寄せの形」を1つずつ詰める
3. 現状コンポーネント（ThinkingPart / CommandToolActivity / ActivityLog 等）とのマッピングを確定
4. MessagePart（Rust/フロント両側）の variant 追加・改廃と変換責務を確定

---

## ありたい姿: 本文 UI ブロックの理想セット（✅ 確定）

本文に並ぶ「ブロック」を以下の **12 種類** に整理する（合意済み）。Claude / Codex の native item をそれぞれ正規化先にマップする方針も併記。

### 会話本体

| # | ブロック | 役割 | Claude 由来 | Codex 由来 | 関連 triage 項目 |
|---|---------|------|------------|-----------|---------------|
| B1 | **Human Message** | ユーザー入力（テキスト＋画像＋@mention） | user message | user message | — |
| B2 | **Agent Text** | エージェントの回答本文（Markdown） | text content_block / text_delta | agent_message | — |
| B3 | **Reasoning / Thinking** | エージェントの思考過程（折りたたみ） | thinking_delta / thinking content_block | reasoning item | **#1** |

### エージェントの行動

| # | ブロック | 役割 | Claude 由来 | Codex 由来 | 関連 triage 項目 |
|---|---------|------|------------|-----------|---------------|
| B4 | **Read Tool** | 情報取得（Read/Glob/Grep/WebFetch/WebSearch 等） | Read/Glob/Grep/WebFetch/WebSearch/ListMcpResources/ReadMcpResource/ToolSearch | web_search item | **#3** |
| B5 | **Command** | コマンド実行＋出力 | Bash | command_execution item | **#5**, **#8**（delta集約） |
| B6 | **File Change** | ファイル編集（diff/preview） | Write/Edit/MultiEdit/NotebookEdit | file_change item | **#6** |
| B7 | **MCP Tool** | MCP サーバー経由のツール呼び出し | mcp__* ツール | mcp_tool_call item | **#7** |
| B8 | **TODO List** | エージェントの計画／進捗チェックリスト | TodoWrite | todo_list item | **#2** |
| B9 | **Task Status** | サブタスク（task_started/progress/notification/updated） | task_started / progress / notification / updated | （なし） | **#11**（task_updated 反映） |

### 失敗 / 制御

| # | ブロック | 役割 | Claude 由来 | Codex 由来 | 関連 triage 項目 |
|---|---------|------|------------|-----------|---------------|
| B10 | **Error** | エラー表示（折りたたみ） | SDK 境界エラー / 既存 Error | error item | **#4** |
| B11 | **Permission** | 許可要求／拒否表示 | tool 許可要求 / permission_denied | （なし） | **#13** |
| B12 | **System Notification** | システム通知（compaction / hook / files_persisted 等） | compact_boundary / hook_* / files_persisted / local_command_output | （なし） | — |

---

## 各ブロックの役割・見た目の方針（叩き台）

> 各ブロックは「1つずつ確認」フェーズで詳細を詰める。以下は本家寄せ＋共通UI統一の原則から導いた叩き台。

### B1 Human Message ／ B2 Agent Text（区別方針 ✅ 確定）

両者の視覚的区別を、Claude.ai 風のレイアウトで強める：
- **Human Message**: 淡い背景色のカード（右寄せ）。下部にタイムスタンプとコピーボタンを表示。Markdown 解釈は別途検討（現状は plain text + mention badge + 画像）。
- **Agent Text**: 背景なしの全幅フラット表示。Markdown レンダリングは現状の `AgentMessageContent` を踏襲。
- **マージン**: 両者の間に十分な余白を取り、turn の区切りを視覚的に明確化。

#### B1 細部（✅ 確定）
- 編集ボタン: ⛔ 追加しない（コピーボタンとタイムスタンプのみ）
- Markdown 解釈: ⛔ 加えない（プレーンテキスト + mention badge + 画像のまま）
- 長文コラプス: ✅ Human 独自の閾値（Agent より小さい値）で折りたたむ。具体値は実装時に決定（叩き台: 3000 chars / 50 lines 目安）

#### B2 細部（✅ 確定）
- ストリーミング中表示: ⛔ B2 本体には何も追加しない（静的表示）。生成中の状態表示は B9 task_status や下部 statusbar に任せる。
- Markdown レンダリング / VirtualizedAgentLines / 長文コラプスは現状の `AgentMessageContent` を踏襲。
- **タイムスタンプ＋コピー**: **agent message 単位で 1 セット**。msg.parts のラッパー div を追加し、最後にメタ行を1つだけ置く。コピー対象は msg 内の text part を連結した本文（thinking/tool/permission/system_notification は除外）。実装は `ChatSessionView` の AgentMessageParts ラッパー追加（msg 単位）が必要。

### B3 Reasoning / Thinking（✅ 確定）
- **本家寄せで作り直す対象**（triage #1）。
- 共通 `MessagePart::Thinking` に Codex reasoning も正規化。
- ラベルは "Thinking" 共通（Claude baseline）。
- 本文: ✅ Markdown としてレンダリング（GFM / コードハイライト、B2 と同じ engine 流用）。
- 折りたたみ: ✅ ストリーミング中は自動展開、完了後は自動で折りたたむ。ユーザーの手動展開/折りたたみも可。
- 装飾: ✅ 他ブロックと区別する装飾を入れる。要素:
  - **淡色背景**（muted-foreground 系のサブトル背景でブロック全体を囲む）
  - **先頭アイコン**（Lucide の Brain / Lightbulb / Sparkles のいずれか。実装時に選定）
  - **トーンダウンしたテキスト色**（補足情報感を出す）

### B4 Read Tool（✅ 確定）
- 共通UI: `read` カテゴリの専用 ActivityRow（DefaultToolActivity とは分離）を新設。
- Codex `web_search` を `MessagePart::ToolUse` (tool="WebSearch" 相当) に正規化し B4 に乗せる（triage #3）。
- 見直し要素:
  - **tool 別アイコン**（Lucide: FileText / Search / Filter / Globe など。Read/Glob/Grep/WebSearch/WebFetch/MCP-read/ToolSearch ごとに割り当て）
  - **対象をインライン単一行表示**（JSON を出さず、パス／パターン／URL／クエリを `Grep "foo" in src/` のように1行で読める形に整形）
  - **結果を折りたたみで見せる**（成功時も result を展開可能に。エラー時は現状の CollapsibleError を流用）
  - **成功/失敗インジケータ**（右端にチェック/X アイコン）

### B5 Command（✅ 確定）
- **本家寄せで作り直す対象**（triage #5）。
- Bash / Codex `command_execution` を同UIに集約。
- output delta は **single running block** で逐次更新（triage #8）。
- 見直し要素:
  - **exit_code / status を表示**（completed / failed / in_progress をヘッダーに、exit_code はバッジ。Codex はネイティブ提供、Bash は tool_result パース）
  - **出力を live 更新**（outputDelta を running block 内に逐次追記、ストリーミング表現）
  - **コード表示のスタイルを terminal 風に調整**（モノスペース＋背景色を terminal 風に）
  - **コマンド行と出力の項目分離**（ヘッダー=コマンド文字列、本体=出力。それぞれ別個のコピーボタン）

### B6 File Change（✅ 確定）
- **edit preview を本家寄せで見直したうえで流用**（triage #6）。
- Claude `Write/Edit/MultiEdit/NotebookEdit` と Codex `file_change` を同UIに集約。
- **Diff の見た目は本体 Diff ビューと完全同一コンポーネント**にする。独自簡易 diff（現 `AgentEditPreviewPanel`）は廃し、`DiffViewerSection` / `ShikiDiffViewer` を **再利用**（共通化）。Agent 用に切り出した薄いラッパーで input → originalContent/modifiedContent への正規化のみ Rust 側で行う。
- **「Diff ビューで開く」ボタン**を追加: クリックでメインの Diff ビューに該当ファイルを開く（既存 Diff ビューのルーティング/開閉 API を再利用）。
- **「Open in editor」ボタン**を追加: `DiffToolbar.tsx:62` 等と同じ `invoke("open_in_editor", { filePath })` を呼び、**外部エディタ**（OS のデフォルト / 設定指定のエディタ）で開く。Monaco エディタではない。
- 見直し要素（ヘッダー）:
  - **変更カウントバッジ（+N/-M）**: 追加/削除行数をヘッダーに表示（GitHub PR 風）
  - **kind インジケータ（add/update/delete）**: ファイル名横に A/M/D アイコン
  - **デフォルトで要約表示、クリックで diff 展開**: 一覧では行わず必要時のみ diff を読む
- **複数ファイル変更**: Codex file_change が複数パスを含む場合、Rust 側で **ファイルごとに別の `MessagePart::ToolUse` ブロックに分割**して本文に並べる（1ファイル=1B6ブロック）。フロントは1ファイル単位のレンダリングだけ気にすればよい。

#### B6 細部（✅ 確定）
- ボタン位置: 展開後のフッターに「Diff ビューで開く」「Open in editor」を配置（ヘッダーはサマリのみ）
- diff レンダラ: **`DiffViewerSection` を完全再利用**。`isImage` / `isMarkdown` のファイル種別分岐は本体側のロジックに委譲（Shiki / Markdown / Image を本体と同じ振り分けで使う）。
- 「Open in editor」: `invoke("open_in_editor", { filePath })` を呼ぶ（外部エディタ）

### B7 MCP Tool（✅ 確定）
- **新しい専用UIは作らない**（triage #7）方針を維持しつつ、**MCP は read/write 問わず 1 つの B7 UI に統一**（実装シンプル化のため B4/B5/B6 への振り分けはしない）。
- Claude `mcp__server__tool` と Codex `mcp_tool_call` を同じ ActivityRow で扱う。
- Codex の `server / tool / arguments / result` を Claude の `mcp__server__tool` 命名・input/result 形式に正規化する責務を Rust 側に置く。
- 見た目: B4/B5/B6 と同じトーンの軽い見直しを適用
  - 先頭アイコン（共通の MCP アイコン、例: Plug / Network）
  - 1 行サマリ（`mcp__<server>__<tool>` を読みやすく整形、引数のサマリを添える）
  - 結果折りたたみ
  - 成功/失敗インジケータ

### B8 TODO List（✅ 確定）
- **session で唯一の TODO リスト**として扱う（Claude TodoWrite と Codex todo_list はどちらも同一リストの逐次更新）。
- **置き場所**: チャットのフッターエリアに **折りたたみ常設**。本文には毎更新ごとのブロックは積まない。
- **本文の扱い**: TODO が更新されるたびに「TODO を更新しました（3/7 完了）」のような **1行ログ** を本文に残し、いつ計画が変わったかチャットの文脈から追えるようにする。詳細はフッターを見る前提。
- **本体（フッター）の表示要素**:
  - **チェックボックス（可読専用）**: completed フラグを checkbox アイコンで表示。クリックトグル不可（agent が管理）
  - **進捗サマリ（N/M）**: フッターヘッダーに「3 / 7 完了」を表示。折りたたみ時もこれは見える
  - **状態別スタイル**: completed はテキストをグレーアウト（取り消し線も検討）、未完了は通常表示
- **MessagePart 設計**:
  - フッター本体用に新規 variant（例: `TodoListSnapshot`）を 1 つ持つ。最新スナップショットだけ保持（履歴は持たない）
  - 本文の 1 行ログ用は既存の `TaskStatus` か `SystemNotification` を流用するか、新規 variant を切るか、実装時に判断

### B9 Task Status（✅ 確定）
- 既存 `MessagePart::TaskStatus` を活用。
- **task_updated を bridge で取り込み、同一 task_tool_use_id に patch**（triage #11）。
- 見た目（見直し）:
  - **状態別アイコン**: running=Loader2 (spinner)、completed=Check、failed=AlertCircle、backgrounded=Moon/Pause、stopped=Square（実装時に Lucide から選定）
  - **状態別テキスト色**: failed=destructive、backgrounded=muted、completed=foreground/60、running=foreground
  - **完了時のサマリ表示**: task_notification の summary をヘッダー下に 1 行で表示（task が何を終えたか読める）
- 展開時の子 part 表示は現状の TaskToolActivity の構造を踏襲。

### B10 Error（✅ 確定）
- **対象**: `MessagePart::Error` として本文に独立して並ぶエラーブロック（Codex ErrorItem と Claude SDK 境界エラーが集約）。tool 内エラー（tool_result の isError）は各 tool ブロック内で扱う別件。
- Codex `error` item を Rust 側で `MessagePart::Error` に正規化（triage #4）。
- 見た目: **本家 Claude Code 寄せの専用コンポーネントを新設**（CollapsibleError とは別の独立部品）
  - **枠線**: 赤オレンジ系の 1px ボーダーで囲う
  - **テキスト色**: 同系統の赤オレンジ
  - **背景**: 塗らない（地のまま）
  - **先頭ラベル**: `Error:` テキスト + メッセージ
  - **dismiss ボタン**: 入れない（履歴に残すべき情報）
  - **スタックトレース/詳細**: メッセージ本体だけ常時表示、スタックトレース部分は折りたたみ展開で見せる
- tool 内エラー（B4/B5/B6/B7 で表示中の CollapsibleError）は B10 とは別の見直し対象 → 必要なら別途確認

### B11 Permission（部分確定／詳細詰め中）

#### 集約する kind（4種）
- `"tool"`: 通常の tool 実行許可（Allow / Deny ボタン）
- `"exit_plan"`: ExitPlanMode（plan テキスト＋allowedPrompts）
- `"ask_user_question"`: Claude `AskUserQuestion`（複数選択肢、Allow/Deny は出さない）
- **NEW**: Codex `request_user_input`（triage #17） → 同じ `kind="ask_user_question"` に Rust 側で正規化して合流

#### permission_denied の取り扱い（triage #13）
- bridge で取り込み、決定の理由（`decision_reason`）を保持
- 既存 `Permission(status="denied")` に乗せるか、新 variant を切るかは実装時判断

#### 見た目方針（✅ 確定）
- 「**本家 Claude Code 寄せでトーンを落とす**」。現状の Releash の Permission UI はうるさすぎる。
- **質問と回答の双方をヘッダーで表示**: 回答済みの状態でも、ヘッダーに「質問テキスト」「選択された回答」が両方見える。回答後の振り返りができる。
- **選択肢は折りたたみで隠す**: 回答前は展開（選びやすい）、回答後は自動で折りたたみ。必要なら展開して当時の選択肢を確認可能。
- **シンプルな装飾**: 本家準拠で背景塗りや装飾枠は抑制。タイトル先頭に小さなドットマーカー＋テキスト。番号バッジは右端に控えめに。
- **落とす装飾要素**:
  - 質問全体を囲う外枠（`border + bg-muted/50`）を削除
  - 選択肢の各枠線を削除、hover 色だけで分離感を出す
  - 回答済みの背景色（`bg-green-500/5` / `bg-red-500/5`）を削除（色はテキスト・ボーダーで表現）
- **回答済みはユーザメッセージと同格**:
  - 選択された回答（テキスト）は B1 Human Message と同じスタイル（淡背景カード、右寄せ）で表示する。これにより「人の発話」として一貫した見え方になる。
  - 質問テキスト本体の扱いは別途決める（下記未確定）

#### B11 細部（✅ 確定）
- **回答後の質問テキスト**: エージェント発話として左側に残す（B2 Agent Text と同じスタイルで質問を表示し、その下に回答カードを右寄せ）。会話の流れが自然になる。
- **kind 別アイコン**: 付ける。tool=Shield、exit_plan=ClipboardCheck、question=HelpCircle 等。本家スクショの「• ドット」マーカーに代えて kind が一目で見える。
- **状態別色分け**: 付けない（さらにシンプルに）。状態は「回答カードの有無」「チェックアイコン/X アイコン」のみで表現し、色は使わない。

### B12 System Notification（✅ 確定）
- **対象を `compaction` のみに絞る**。`hook` / `files_persisted` / `local_command_output` は削除（infra/bridge から取り込みを止め、対応する MessagePart variant 経路も整理）。
- 見た目は現状の1行表示（in_progress=⏳ pulse / 完了=✓ + label）を踏襲。文脈切れのサインとしてのみ機能。
- 関連影響:
  - `bridge_common.rs` の `hook_started/hook_progress/hook_response` / `files_persisted` / `local_command_output` 取り込み処理を削除
  - `MessagePart::SystemNotification` の `notificationType` enum から `hook` / `files_persisted` / `local_command_output` を削除（`compaction` のみ残す）

### 横断: Plan モード（triage #18）（✅ 確定）

- **モデル化**: 抽象 `PermissionMode (Ask/Edit/Full)` と独立した `PlanMode (ON/OFF)` トグルを追加。Plan ON 時は PermissionMode 値を内部保持しつつ、Claude には `permissionMode='plan'` を、Codex には Plan mode 起動を送信する。Plan OFF へ戻したとき、Claude では保持していた PermissionMode を再送して**元の権限に復帰**する。
- **UI 配置**: `MessageInput` 内の PermissionMode セレクタ（Ask/Edit/Full）の隣にトグルとして配置。Tab/Shift Tab の cycleMode 経路との関係は実装時に整理。
- **影響範囲**:
  - 抽象 PermissionMode の語彙は変更しない（3値維持）。Plan は別の状態フィールド。
  - Claude SDK の `permissionMode` 値マッピング（`permission_flags.rs`）に Plan 経路を追加
  - Codex SDK の Plan mode 起動方法は要追加調査（実装時）
  - `MessagePart::Permission` の `exit_plan` kind は Plan モード時に agent から提案されるので、Plan OFF への遷移は ExitPlanMode 承認時にも起こる（既存挙動を整合させる）

---

## 削除されるブロック（triage の見送り由来）

| 項目 | 内容 |
|------|------|
| Codex `goal` row | BoundSessionChat / ChatSessionView から完全削除。infra/listener 側の goal 入り口も整理（triage #14） |
| Codex `runtime status` パネル | isStatusOpen / codexRuntimeStatus prop / パネル本体まで削除（triage #15） |
| tool_progress / tool_use_summary / thinking_tokens | bridge に取り込まない（triage #9 / #10 / #12） |

---

## 議論ポイント（B1 から順に詰める）

### 確認中

- **B1×B2 視覚的区別（最優先）** — Human Message と Agent Text が「マークダウンやコード表示と差がなく非常に見づらい」という指摘あり。視覚的区別を強めて両者を一目で見分けられるようにする方向で要詳細決定。

### 各ブロック個別

- B1 Human Message の見た目（上記 B1×B2 区別と一緒に詰める）
- B2 Agent Text の見た目（同上）
- B3 Reasoning/Thinking の見た目 — 本家寄せの具体形（折りたたみ装飾、ストリーミング表現、ラベル位置）
- B4 Read Tool の見た目 — Claude Read/Glob/Grep の現 DefaultToolActivity を本家寄せで見直すか
- B5 Command の見た目 — terminal 風表示、exit_code/status、output buffer の live 更新
- B6 File Change の見た目 — diff の表示形式、複数ファイル変更時のまとめ方
- B7 MCP Tool の見た目 — DefaultToolActivity 共通だが、MCP 特有の表示要素（server 名等）があるか
- B8 TODO List の見た目 — チェックリストの装飾、進捗表示
- B9 Task Status の見た目 — running/completed/failed/backgrounded の見せ分け
- B10 Error の見た目 — CollapsibleError の現状で十分か
- B11 Permission の見た目（denied 含む） — 表示先（新 variant か既存 Permission への相乗りか）
- B12 System Notification の見た目 — compaction / hook / files_persisted の見せ分け

### 横断（✅ 確定）

- **ブロック間の余白／グルーピング**: turn 境界を **padding / margin だけで表現**。Human Message の上に広めの余白、同一 turn 内のブロックは狭めの余白で並べる。区切り線や背景色は使わない。

---

## 実装引き渡し（Codex 用）

このセクションの詳細（MessagePart 最終形、削除タスク、実装順序、保留中の事前調査、テスト方針、scope-out、DoD）は **[goal.md](./goal.md)** に集約した。
Codex には `goal.md` の内容をそのまま `/goal` で渡すことを想定している。本 design.md は UI 設計の正本として参照される。
