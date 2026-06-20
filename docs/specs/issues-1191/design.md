# Design

本書は `requirements.md` / `behavior.md` を実装方針へ落とし込む設計文書である。本 Issue は不具合修正（回帰）であり、設計の中心は「メモリ枯渇の原因経路の特定」と「特定経路への最小限の是正」である。

> 本設計の出発点（調査スタンスの転換）: 当初は「v0.3.53 を安定基準とし、v0.3.53..HEAD の静的差分から回帰を特定する」方針で着手したが、(1) 初期仮説の経路は v0.3.53 と HEAD で実質同一コードであること、(2) Codex の app-server 移行は v0.3.53 より前（#1141）で既に v0.3.53 に含まれること、(3) 「v0.3.53 では発生しない」という前提自体が未検証で誤りの可能性があること、を踏まえ、**導入時期や git 差分に依存せず「会話・ターン・出力が増えるほど解放されずに増え続ける／巨大化する構造」をコードから広域に洗い出し、それを原因候補として実測で確定する**方針へ転換した。広域調査の結果は「調査結果サマリ」に示す。

## 概要

Codex backend との会話で発生するメモリ枯渇クラッシュを解消する。設計は次の 2 段構えとする。

1. **原因特定フェーズ（診断）**: コードから特定した「構造的に無制限増大しうる経路」（後述の候補 C1〜C10）を、再現シナリオ + メモリ計測で実測し、OOM を支配的に駆動する経路を確定する。可能なら v0.3.53 と HEAD を含む複数ビルドで比較するが、**「v0.3.53 が安定」という前提には依存しない**（前提が崩れても診断が成立するよう、絶対的なメモリ推移そのものを基準にする）。
2. **是正フェーズ（修正）**: 本 ISSUE では、振る舞い不変・低リスクな**純粋削減（A群: C2 / C3 / C4 / C7 / C8 / C9）をまとめて実施**する。同じ出力をより少ないメモリ／少ない複製で生成する是正に限定し、外部観測可能な出力は不変に保つ。

### スコープの分離（本 ISSUE と別 ISSUE）

ライフタイム/解放の設計判断を要する候補（B群: C1 / C6 / C10）は、振る舞い（表示・永続化・履歴復元）に影響しうるため本 ISSUE から分離し、個別 ISSUE で対応方針を検討する。

- **本 ISSUE (#1191) のスコープ**: A群 = **C2 / C3 / C4 / C7 / C8 / C9**（純粋削減）。C8 は唯一の実回帰差分のため優先度高。
- **C1 → #1194**（ターン完了時に `streaming_parts` を解放）
- **C6 → #1195**（フロント全メッセージ保持の退避/仮想化）
- **C10 → #1196**（terminal 後の workflow execution 解放）

方針: まず A群を実装し、実測でクラッシュ・メモリ増大が解消するか確認する。解消しきらない場合は、診断で支配的と判明した B群 ISSUE（#1194/#1195/#1196）を後追いで対応する（「実装して直らなければ再修正を追加する」運用）。

本設計の受け入れ基準は、本 ISSUE 単体では「A群削減によりメモリ増幅が有意に低減し、既存の振る舞いに退行がないこと（既存テスト・lint green）」とする。再現シナリオでのクラッシュ完全解消は、必要に応じて B群 ISSUE と合わせて達成する。

## 調査結果サマリ（原因候補の切り分け）

コードを広域に調査し、無制限増大しうる構造を抽出した。これ自体が `requirements.md` の「原因経路の特定・記録」要件の一部を満たす中間記録であり、診断フェーズ完了時に最終結果（実測で確定した主因）へ更新する。各候補は実コードで裏取り済み。

> 時系列で除外済みの事項: Codex の app-server 移行 (#1141, `60f798ca9`) は v0.3.53 (`f7fbb925`) の祖先であり、v0.3.53 に含まれる。よって「app-server 化そのもの」は v0.3.53→v0.3.55 の回帰原因にはならない。また `src-tauri/Cargo.toml` の依存は当該区間で変更なし。`codex_app_server.rs` / `ws_bridge.rs` / フロントの streaming 受信処理も v0.3.53..HEAD では実質同一であり、単独の v0.3.55 新規混入経路とは見なしにくい。

### 無制限増大しうる構造の候補

| ID | 経路 | 場所（裏取り済み） | 増大の効き方 | 確度 |
|---|---|---|---|---|
| **C1** | `streaming_parts` がターン完了時に解放されない（clear は次ターン開始時の `reset_streaming_state_for_new_turn` のみ）。通常 turn 完了では `AgentProcessMap` から process は削除されず、明示 close まで `AgentProcess` が残るため、次ターンが来ない session では完了ターン分の全 parts が常駐し続ける。post-turn のバックグラウンドイベントも同じバッファへ追記され得る | `bridge_common.rs` 構造体 L85、ターン完了 `run_turn_complete_transition_locked` L1236（clone のみ・clear なし）、turn start reset L4727、process 登録 L4767-4795、明示 close remove L4808-4811、post-turn 追記 L2888-2950 | 会話全体（アイドル時も直前ターン分常駐）＋ post-turn 追記分＋複数 session | 高 |
| **C2** | flush 毎（L1053）・ターン完了毎（L1236）・persist 毎（L2725 付近）に累積 `streaming_parts` 全体を `.clone()`。単一ターンで大量出力時、ライブ常駐に加え flush 頻度分の全サイズ複製が重なりピークメモリが累積サイズの数倍に達しうる | `bridge_common.rs` `prepare_streaming_flush` L1049-1053、L1236、L2725 付近 | 単一ターン（大量出力で顕著） | 高 |
| **C3** | `push_or_update_tool_result` が同一 ToolResult の delta 更新時に growing content 全体を `parts[index].clone()` で返す。Codex の `command_output_delta` は同一 tool result に追記されるため、大量 stdout/stderr で「成長済み ToolResult の全 clone」が delta ごとに発生しうる | `bridge_common.rs` `push_or_update_tool_result` L1819-1856、`codex_app_server.rs` `command_output_delta_message` L595-599 | 単一ターン（巨大 command output で顕著） | 高（v0.3.53 にも存在） |
| **C4** | workflow run の `get_run_log` が state 変化 emit のたびに全イベントを disk から再読込・再構築（`read_events` → `event_draft` で `payload.clone()` → `collect`）。アクティブ run で emit 頻度が高いと run 長に対し O(N) 確保 × 高頻度 ＝ O(N²) 的なアロケーション churn でピークが跳ねる | `usecase/workflow/query_service.rs` `read_events`、`usecase/workflow/event_draft.rs` L24/L66、controller `get_run_log`、フロント `useWorkflowRunLog`（refreshKey=updatedAt） | 会話/run 全体（churn 由来のスパイク） | 中 |
| **C5** | フロント agent チャットが cumulative streaming payload を `SET_STREAMING_MESSAGE` で丸ごと置換し、描画側でも `buildToolPairings` / `getTextContent` 等で parts 全体を繰り返し走査する。remote UI も `agent_stream_sync` の `parts` を丸ごと置換するため、Rust 側の cumulative clone と WebView/remote 側の保持・再処理が同時に効く | `src/hooks/useAgentSdkListeners.ts` L358-395、`src/hooks/agentChatReducer.ts` L339-353、`src/components/panels/AgentChatPanel/ChatSessionView.tsx`、`src/remote/components/RemoteAgentPanel.tsx` L334-360 | 単一ターン（delta 多）＋会話全体（webview / remote メモリ） | 中（v0.3.53 から同型） |
| **C6** | フロントが全セッション・全メッセージを `sessionsById` に保持し続け、退避・仮想化なし | `src/hooks/agentChatReducer.ts` ADD_MESSAGE 系 | 会話全体（webview メモリ） | 低〜中（要再確認） |
| **C7** | fileChange / patch update で `diff` を ToolUse input と ToolResult content の両方へ載せ、さらに `changes: [change.clone()]` も保持するため、巨大 diff が複数表現で重複する | `codex_app_server.rs` `file_change_tool_messages` L427-455、`file_change_patch_messages` L601-606 | 単一ターン（巨大 diff / patch update） | 高（v0.3.53 にも存在） |
| **C8** | #1167 後の workflow turn complete 通知が、通常 Codex セッションでも `is_session_running` 判定の前に `final_parts` から Text 全体を `content.clone()` して `final_text_parts` を構築し、gateway 側で再び `MessagePart::Text` に包み直す。v0.3.53 では `engine.is_running()` の後に `&final_parts` を渡していた | `bridge_common.rs` `spawn_workflow_turn_complete_notification` L4973-4999、`usecase/workflow/turn_complete.rs` L31-39、`runtime_command_gateway.rs` L294-320 | ターン完了時のピーク（巨大 Text で顕著） | 高（v0.3.53..HEAD の実差分） |
| **C9** | `SessionStore` が `ChatSession` 全体を cache に保持し、初回 `ensure_loaded` で sessions dir の JSON を全件 read/parse して cache に載せる。さらに `get_session` / `save_session` / persist 時に全体 clone + JSON serialize を行う。会話履歴・parts が大きいほど Rust 側の基礎メモリと保存時ピークが増える | `usecase/agent_session/session/store.rs` cache、`ensure_loaded` L470-543、`list_sessions_filtered` L133-155、`get_session`、`save_session`、`persist_and_update_cache` | 会話全体＋全 session cache 常駐＋保存時ピーク | 中（v0.3.53 にも存在） |
| **C10** | workflow runtime の `executions` が `run_id -> WorkflowExecution` を保持し、terminal 化後も `WorkflowExecution` 本体を通常経路では削除しない。`WorkflowExecution` は `step_history` / `step_outputs` / `workflow_variables` / `workflow_definition` を持ち、`to_snapshot()` でも全体 clone するため、workflow run が大きいほど常駐量と emit 時ピークが増える | `runtime_engine_impl.rs` `executions` L161-168、terminal 経路 L1948-1966 / L2730-2741 / L3316-3329（refs cleanup と broadcast のみ）、`execs.remove` は起動失敗系 L652 のみ、`workflow_execution/mod.rs` L18-36 / L167-199 | workflow run 全体（terminal 後の常駐）＋状態 emit 時ピーク | 中（v0.3.53 旧 `workflow/engine.rs` にも同型） |

補足: emit/broadcast 失敗時、`apply_streaming_emit_result`（L1066-1094）は pending 会計（`pending_stream_part_count` / `pending_stream_bytes`）を**意図的にリセットしない**（cumulative ペイロードを次 flush で再送するため）。これ自体は会計値の保持に過ぎないが、失敗が続く間も `streaming_parts` 本体は delta ごとに増え続ける（C1/C2 を増幅）。`requirements.md`「emit 失敗時に無制限に積み増される経路」はこの増幅として扱う。

### 広域追加調査で低優先に落とした経路

- **PTY / remote access 経路**: #1175 で PTY 周辺も clean architecture へ移行しているが、PTY 出力は `ws_bridge.rs` の ring buffer が 64 KiB、`pty_session/services.rs` の pending UTF-8 buffer が 16 KiB に制限され、`PtySessionRuntimeGateway` も終了後 cleanup を持つ。Codex 会話の `streaming_parts` のように本文全体を無制限に保持する構造は見つからないため、本 Issue の主因優先度は低い。
- **WebSocket queue 経路**: `AgentStreamSync` は通常メッセージ用の unbounded channel ではなく `(session_id, message_id)` ごとの latest slot へ coalesce するため、ストリーミング payload が queue として無制限に積まれる構造ではない。ただし latest slot には「累積済みの full snapshot 1 個」が残るため、C2/C5 の一部として計測する。
- **AgentStatusCenter 経路**: `update_workflow_snapshot` が保持する workflow 情報は `execution_id` / `agent_state` / `last_activity_at` の小さい集約スナップショットであり、workflow 本体や event log は保持しない。`list_sessions()` による clone は status DTO の clone であり、Codex本文の支配的保持経路とは見なしにくい。
- **Codex app-server request state**: `AppServerBridgeState` が保持する可変 map は `pending_approval_methods` のみで、JSON-RPC request/response 全体を蓄積する別 map は見つからない。approval 未応答時の entry 残存は v0.3.53 以前から同型で、巨大 output/diff に比例して増える経路ではないため、本 Issue の主因候補からは外す。
- **workflow runtime 経路**: `WorkflowRuntimeService.executions` は terminal 後も `WorkflowExecution` 本体を通常削除していないため、workflow run 固有の常駐候補として C10 に残す。一方で、v0.3.53 旧 `workflow/engine.rs` にも同型の `executions` map と cleanup-only terminal 経路があるため、v0.3.55 だけの新規混入原因ではなく、workflow 利用時の構造的候補として扱う。

### v0.3.53 前提を疑うためのタグ横断確認

| 確認対象 | v0.3.53 | HEAD / v0.3.55 側 | 判断 |
|---|---|---|---|
| C1/C2 `streaming_parts` の reset/clone | `streaming_parts.clear()` は turn start のみ、`consolidate_parts(proc.streaming_parts.clone())`、turn complete の `proc.streaming_parts.clone()` が存在 | 同じ行相当が存在 | v0.3.53 でも巨大 turn の常駐・clone ピークは起こり得る |
| C3 ToolResult delta clone | `push_or_update_tool_result` が `parts[index].clone()` を返す | 同型 | v0.3.53 でも大量 command output で clone ピークが起こり得る |
| C7 file diff 重複 | `diff` を ToolUse input、`changes: [change.clone()]`、ToolResult content に載せる | 同型 | v0.3.53 でも巨大 diff の重複保持が起こり得る |
| C8 workflow turn complete 追加 clone | `engine.is_running()` の後に `&final_parts` を渡す | `is_session_running` 相当の判定前に `final_text_parts` として Text 全体を clone | v0.3.55/HEAD 側の明確な追加ピーク要因 |
| C9 SessionStore cache/serialize | `ensure_loaded` が session JSON を全件 read/parse して cache へ投入し、`cache: HashMap<String, ChatSession>`、`get_session().cloned()`、`serde_json::to_string_pretty(session)`、`cache.insert(..., session.clone())` が存在 | 同型 | v0.3.53 でも全 session 履歴の常駐と cache/clone/serialize ピークは起こり得る |
| C10 workflow execution 保持 | 旧 `workflow/engine.rs` に `executions: Mutex<HashMap<String, WorkflowExecution>>` と terminal 時 refs cleanup + broadcast が存在 | clean architecture 後も同型 | workflow execution 本体の保持は v0.3.55 だけの新規混入ではない |

**含意**: 単一ターンでのクラッシュ報告は C1+C2+C3+C7（巨大ターンの常駐＋全複製ピーク＋ToolResult/diff の重複）で v0.3.53 でも説明可能である。HEAD/v0.3.55 では C8 が追加のピークメモリ要因として加わるため、「v0.3.53 は完全に安定、v0.3.55 だけが壊れた」と断定するには実測が必要。長時間会話でのクラッシュは C1・C4・C5・C6・C9 の累積で説明可能で、workflow run を併用している場合は C10 も加わる。どれが支配的かは診断フェーズの実測で確定する。

## 変更対象

診断フェーズの結果により確定するが、是正フェーズで触れる可能性が高い範囲は以下に限定する想定。

- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` — ストリーミングバッファのライフタイム・解放（最有力の防御対象）。
- `src-tauri/src/usecase/agent_session/session/` 配下 — 会話履歴の保持・clone 経路（O(N²) 緩和を行う場合）。
- `src-tauri/src/usecase/workflow/` / `src-tauri/src/adaptor/gateway/workflow/` 配下 — workflow 進行が原因経路に含まれる場合のイベント読込経路・runtime 実行状態保持。
- 診断専用の補助（後述）はリポジトリに恒久コミットしない（Non-goal: 常設プロファイラの製品化）。

> 仮定: ストリーミング表示・イベント永続化・workflow 進行の**外部仕様は一切変更しない**。本修正はメモリ挙動の是正のみを目的とし、表示内容・永続化イベント・進行状態の出力は不変に保つ（`requirements.md` Constraints）。

## アーキテクチャと責務分割

クリーンアーキテクチャの層構成（`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`）を維持し、層をまたぐ再設計は行わない。メモリ是正は原則として「データを生成・保持する側の層」に閉じる。

- ストリーミングバッファの蓄積・解放は `infrastructure/agent_session/runtime`（バッファの所有者）に閉じる。
- 会話履歴の保持・キャッシュは `usecase/agent_session/session`（履歴の所有者）に閉じる。
- workflow イベントの読込・投影は `adaptor/gateway/workflow` / `usecase/workflow` に閉じる。
- いずれも `protocol` / フロントエンドへ流れる出力の形は変えない（責務境界を越えない）。

> 制約: 回帰解消に不要なリファクタ・revert を混在させない（`requirements.md` Constraints / Non-goals）。#1167/#1175 の全面巻き戻しは行わない。

## データモデルまたは型

新規の永続データ型・プロトコル型は追加しない。是正は既存フィールドのライフタイム管理に閉じる。是正候補として関係する既存型は以下。

- `AgentProcess`（`bridge_common.rs` L85〜）
  - `streaming_parts: Vec<MessagePart>` — ターン内累積、ターン開始時のみ `clear`。
  - `pending_stream_part_count: usize` / `pending_stream_bytes: usize` — flush 判定用のキャップ会計。
  - `pending_messages: VecDeque<PendingMessage>` — キュー済み入力。
- 会話履歴: `usecase/agent_session/session` のメッセージ `Vec` とそのキャッシュ。
- workflow イベント: `event_draft.rs` / `event_repository.rs` が読み込むイベント列。
- workflow runtime: `WorkflowRuntimeService.executions` と `WorkflowExecution`（`step_history` / `step_outputs` / `workflow_variables`）。

防御的是正を行う場合でも、上記の**型シグネチャ・シリアライズ表現は変更しない**ことを原則とする（永続化・プロトコル互換のため）。

## 処理フロー

### 診断フェーズ

1. **再現シナリオの確立**: `behavior.md` の 2 シナリオ（(A) 長時間・多ターン会話、(B) 単一ターンで大量 delta/ツール出力/diff）を、Codex backend で再現する固定プロンプト・手順として用意する。(B) は単一ターンクラッシュ報告がある分、一次再現シナリオとして優先する。
2. **計測（前提非依存）**: HEAD ビルドで上記シナリオを実行し、RSS 推移を計測（macOS は Instruments / `footprint` / `ps` 系、または一時的なヒーププロファイル）。「会話量・出力量に比例して際限なく増加するか／クラッシュするか」を**絶対的な推移として**判定する。これは「v0.3.53 が安定」という前提に依存しない。可能なら v0.3.53 / v0.3.54 / HEAD を比較するが、v0.3.53 でも増大が観測される場合は「回帰ではなく従来から存在する構造的問題」として扱い、原因候補の優先度を実測値で決める。
3. **候補の切り分け（実測で支配経路を特定）**: 候補 C1〜C10 のうち、どれが OOM を駆動しているかを切り分ける。手段の例:
   - C1/C2: 単一巨大ターン (B) でターン進行中・完了直後・次ターン開始後の RSS を計測し、ターン完了で解放されず常駐するか、flush 頻度に比例してピークが跳ねるかを確認。
   - C3: 巨大 command output を生成し、delta 追記時に growing ToolResult の clone がピークを押し上げるかを確認。
   - C4: workflow run をアクティブにした状態としない状態で (A)/(B) を比較し、`get_run_log` 再読込頻度の寄与を切り分け。
   - C5/C6: フロント（webview）側メモリと Rust プロセスメモリを分離して観測し、どちらでクラッシュしているかを判定。
   - C7: 巨大 file diff / patch update を生成し、diff の重複保持が command output と別のピークを作るかを確認。
   - C8: Text のみの巨大応答で v0.3.53 / v0.3.55 / HEAD を比較し、#1167 の turn complete 追加 clone が閾値差を作るかを確認。
   - C9: 大きな履歴を持つ session JSON を複数用意し、初回 session 一覧取得後の Rust 側 RSS、個別 get/save 時の clone、JSON serialize のピークを確認。
   - C10: workflow run を複数回完了させ、terminal 後に `WorkflowExecution` 相当のメモリが解放されるか、run 数・step output 量に比例して常駐し続けるかを確認。
4. **（任意）二分探索**: 「特定バージョンでは増大しない」ことが実測で確認できた場合に限り、`git bisect`（判定基準は実測メモリ挙動）で混入コミット（#1167 / #1175 / その他）を特定する。確認できない場合は bisect に固執せず、実測で確定した支配経路の是正を優先する。
5. **記録**: 確定した支配経路（と、判明すれば混入元）を本 `design.md` の調査結果サマリへ追記する（`behavior.md` の観測点を満たす）。

### 是正フェーズ

本 ISSUE では A群（純粋削減）をまとめて実施する。各是正は「外部に出る出力（表示・永続化イベント・workflow 進行状態）を変えず、メモリ／複製のみを減らす」ことを前提とする。

1. **C8（workflow turn complete の追加 clone 削減）— 優先度高（唯一の実回帰差分）**: `spawn_workflow_turn_complete_notification`（`bridge_common.rs` L4973-4999）で、workflow 実行判定の前に `final_parts` から Text 全体を `content.clone()` して `final_text_parts` を構築している。v0.3.53 は `engine.is_running()` ガード後に `&final_parts` を借用で渡していた。実行判定を `final_text_parts` 構築前に戻す（または借用/遅延構築にする）ことで、通常 Codex セッションで不要な Text 全体コピーを発生させない。
2. **C2（複製削減）**: flush/persist のたびの `streaming_parts.clone()`（L1053 / L1236 / L2725 付近）を、外部出力を変えずに削減（差分のみの consolidate、参照渡し化、確定済み部分の事前 consolidate など）。ピークメモリ削減が目的で、送出結果は不変。
3. **C3（ToolResult 更新の複製削減）**: command output delta 更新時に growing ToolResult 全体を毎回 clone しない（`push_or_update_tool_result` L1856 の `parts[index].clone()`）。表示・永続化上の最終 ToolResult は不変に保ちつつ、delta 通知と累積保持の責務を分ける。
4. **C7（diff 重複の削減）**: ToolUse input / ToolResult content / raw `changes` のどこに diff を保持するかを見直し、外部表示を変えずに巨大 diff の重複を避ける（`codex_app_server.rs` `file_change_*`）。
5. **C4（run_log 再読込の緩和）**: state 変化 emit ごとの全イベント再読込を、外部出力を変えずに緩和（増分読込・キャッシュ・emit 由来データの再利用）。
6. **C9（SessionStore clone/serialize 緩和）**: 保存時の clone 粒度や cache 更新を見直し、全履歴の再複製ピークを減らす（出力・永続化形式は不変）。
7. 各是正は「外部観測可能な振る舞いを変えない」ことを単体テストで固定する。実装は寄与の大きい順（C8 → C3/C7 → C2 → C4/C9）に進め、各是正後に再計測する。

> B群（C1 / C6 / C10）は別 ISSUE（#1194 / #1195 / #1196）で対応。本 ISSUE では扱わない。A群実装後の実測でクラッシュが解消しきらない場合に、診断結果に基づき後追いで着手する。
>
> C5（フロント cumulative 置換・再処理）は、`rust-first-logic` 方針上ロジックをフロントに足さず Rust 側の供給最適化（A群 C2/C3 等）で間接的に軽減される範囲に留め、webview 固有の保持問題は C6（#1195）として扱う。

### 検証フェーズ

1. 修正後ビルドで再現シナリオ (A)/(B) を実行し、クラッシュしないこと・RSS が会話量に対し頭打ちになることを確認。
2. 比較基準が成立する場合（特定バージョンで増大しないことが実測できた場合）、そのビルドと修正後ビルドでメモリ推移が同等水準に収まることを確認。成立しない場合は「修正前 HEAD と修正後で増大が解消したこと」を基準にする。
3. 既存テスト・lint を green に保つ。

## エラー処理

- **emit/broadcast 失敗**: 失敗時もバッファのキャップ会計（`pending_stream_*`）と解放が必ず進む経路を保証する。失敗を握りつぶしてバッファを保持し続ける分岐を作らない（`requirements.md`「emit 失敗時に無制限に積み増される経路が存在しないこと」）。
- **ターン異常終了/キャンセル**: 正常完了と同様にターン由来の一時バッファを解放する（持ち越し防止）。
- 既存のエラー型・エラー伝播の構造は変更しない。新たなエラー分類は追加しない。

## テスト方針

`requirements.md` / `behavior.md` は外部観測可能なメモリ挙動を基準とするため、テストは「振る舞いの不変」と「解放の確定」を単体で固定し、実メモリ挙動は手動検証で担保する二層構成とする。

### 自動テスト（`cargo test` / `pnpm test`）

- **解放の確定（ロジック単体）**: ターン完了で `streaming_parts` / `pending_stream_*` が解放（長さ 0・会計 0）になることを検証するテストを追加（既存 `reset_streaming_state_for_new_turn_clears_all_coalescing_state` L6730 を、ターン完了起点でも担保する形へ拡張）。
- **emit 失敗時の解放**: emit クロージャが失敗を返すケースで pending が無制限に増えないこと（解放 or キャップが効くこと）を検証。
- **振る舞い不変（回帰防止）**: 是正前後で、ストリーミング consolidate 結果・永続化 parts・workflow イベント投影の出力が一致することをテストで固定（出力スナップショット相当）。
- 既存テストは変更せず green を維持。テスト期待値を実装に合わせて書き換えない（仕様優先）。

### 手動・経験的検証（自動化しない）

- 再現シナリオ (A)/(B) による v0.3.53 / 修正後の RSS 推移比較。常設のモニタリング/プロファイラはコミットしない（Non-goal）。手順は `design.md`（または検証メモ）に記録し再実行可能にする。

## リスクと代替案

- **リスク: 「回帰」かどうか自体が不確実**。「v0.3.53 で発生しない」は未検証で、誤りの可能性がある。仮に v0.3.53 でも増大するなら、これは回帰ではなく従来から存在する構造的問題（C1〜C7/C9/C10）であり、bisect は空振りする。
  - 緩和: 診断を「絶対的なメモリ推移」基準にし、特定バージョンの安定性に依存しない。bisect は前提が実測で成立したときのみ行う。
- **リスク: A群削減だけではクラッシュが止まらない**。支配的要因が B群（C1/C6/C10, #1194/#1195/#1196）にある場合、本 ISSUE の純粋削減では頭打ちにならない恐れ。
  - 緩和: A群実装後に実測し、解消しきらなければ診断で支配的と判明した B群 ISSUE を後追いで対応する（運用合意済み）。本 ISSUE の達成基準は「A群削減による有意な低減＋退行なし」とし、クラッシュ完全解消は B群と合わせて達成する。
- **リスク: 「純粋削減」が実は振る舞いを変える**。clone 削減・キャッシュ化が、表示・永続化・進行状態の出力を意図せず変える恐れ。
  - 緩和: 各是正に「出力不変」スナップショットテストを付け、A群を一括で入れる前後で出力一致を確認する。
- **リスク: フロント（webview）側が主因の場合**。Rust プロセスではなく webview のメモリで落ちている可能性（C5/C6）。
  - 緩和: 診断で Rust プロセスメモリと webview メモリを分離観測する。webview 固有の保持問題は C6（#1195）で扱う。
- **代替案 A: 該当リファクタ（#1167/#1175）の部分 revert**。実測で回帰が確認でき、かつ最小是正より revert が安全な場合に限り検討。全面 revert は Non-goal。
- **代替案 B: 純粋な防御的上限のみ（原因非特定のまま緩和）**。`requirements.md` は「原因経路の特定・記録」を要件にしているため単独では不可。原因特定と併用する補助に留める。

## 仮定

- メモリ挙動は RSS 推移およびクラッシュ/強制終了の有無として外部観測する。「際限なく増加しない」＝「会話量・ターン数に対し概ね頭打ち」と解する。
- 計測では Rust プロセスのメモリと webview（フロント）のメモリを可能な限り分離して観測する。
- 「機能的振る舞いが変化しない」＝ ストリーミング表示・永続化イベント・workflow 進行状態の外部出力が修正前後で一致すること。
- 原因特定結果は本 `design.md`（調査結果サマリ）へ追記して記録する（`behavior.md` の観測点を満たす）。
- 是正は Codex 会話のメモリ枯渇解消を主目的とし、共通経路修正で他 backend が副次的に改善するのは可、ただし他 backend 固有問題は追わない（Non-goals）。
- 候補 C1〜C10 は実コードで構造として裏取り済みだが、「どれが OOM を支配的に駆動するか」は診断フェーズの実測で確定する（本書時点では未確定）。

## Open Questions

なし（C1/C6/C10 の解放設計は別 ISSUE #1194/#1195/#1196 へ移管。本 ISSUE は A群純粋削減に限定して着手する）。
