# issues-1023: Workflow Engine Evolution - Workflow Panel / Command Center

関連: [GitHub Issue #1023](https://github.com/siro33950/releash/issues/1023) / マイルストーン [07] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) / 先行 [issues-1011](./issues-1011.md) / [issues-1013](./issues-1013.md) / [issues-1015](./issues-1015.md) / [issues-1019](./issues-1019.md)

## 要求

**種別**: 新機能

**ゴール**: workflow run を chat tab 群から独立した第一級の UI モデルとして扱える「Workflow panel / Command Center」を中央エリア側に確立し、利用者が多数の chat session を切り替えることなく、現 worktree に紐づく workflow run の active 状態・履歴・経緯・step ごとの詳細・step での対話内容を一箇所で inspect できるようにする。具体的には以下が満たされること。

- 中央エリアに AgentChat / Workflow の表示モード切替が存在し、利用者は Workflow モードへ切り替えると、現 worktree に紐づく workflow run の状況をこの一つのパネルで把握できる。
- Workflow panel に active run と run history が表示され、利用者は現 worktree で進行中の run と過去 run の両方を、別画面に遷移せずに一覧できる。
- run event の timeline が表示され、利用者は当該 run が「いつ / どの node で / どんな事実が起きたか」を時系列で観測できる。timeline は read-only 観測経路（[05]）で確立された `WorkflowEvent` 列の事実を主語として表示し、CLI 経由 mutation 要求の事実（[06] で追加された typed event）も含めて経路非依存に観測できる。
- step detail view が表示され、利用者は timeline 上の任意の step（node 実行）を選択して、その step の入出力・遷移結果・所要時間などの詳細を確認できる。
- step conversation transcript が表示され、利用者は当該 step が agent node である場合の対話履歴を、chat tab を開かずに Workflow panel 内で読める。
- Workflow panel 内の approval / reject / abort ボタンは [04] / [06] で確立された run_id 主語の typed command boundary（`WorkflowCommand::ApproveNode` / `RejectNode` / `AbortRun` → `WorkflowEngine::dispatch_external`）に通る。経路としては既存 Tauri command（`approve_workflow_step` / `reject_workflow_step` / `abort_workflow`）をそのまま再利用し、engine から見て CLI 経路（[06]）と UI 経路は同一 dispatch 入口に合流する。本 issue では「UI 専用の新 file-direct 経路」は導入しない。
- workflow step として起動された chat session は、自由対話の chat tab と同格には表示されない。step の対話は Workflow panel の step conversation transcript を正規の閲覧経路とし、tab bar 上に workflow step ごとの chat tab が氾濫する状態を解消する。
- 観測対象は現 worktree に紐づく workflow run に限定する。他 worktree の run を横断表示する責務は本 issue では持たない。

**スコープ外**: 以下は本 issue の対象外。

- 新規 run 起動 CLI / UI（plan doc [06] 外で別 issue）。Workflow panel から新たに run を kick する UI は本 issue では扱わない。
- structured output 提出 CLI / UI（[08]）。step conversation transcript 上で `<workflow_output>` 提出 UI を追加することは本 issue では行わない。
- bash node 実行系統（[13]）。
- Workflow template marketplace / external triggers / Web UI 移植（plan doc「採用しないもの」）。
- main-agent narrator への typed event 配信と user-facing report の整備（[16]）。
- workflow map / DAG 表示（plan doc により後回し）。
- 他 worktree の run の横断ダッシュボード化。
- pending command store / watcher / 「CLI 要求の事実」typed event の責務拡張（いずれも [06] の責務範囲に閉じる）。
- Remote セッション側（`src/remote/`）への Workflow panel 移植。本 issue はデスクトップアプリの中央エリアに閉じる。

**現状温存**: 既存の自由対話 chat tab（workflow step として起動されたものではない通常 chat）の表示・操作経路は本 issue では破壊しない。既存 UI approval / abort 経路（`approve_workflow_step` / `abort_workflow` Tauri command と、それを呼ぶ既存フロントエンド経路）も破壊しない。本 issue は Workflow panel から approval を発行する際の到達先として既存 Tauri command を再利用する。Source Control / Review panel など右パネルの既存表示モードは温存され、Workflow モードは中央エリア上で AgentChat と並列の切替先として追加される。本 issue は既存 event log / agent chat session ストアに既に保存されている内容を Workflow panel に表示するのみであり、step の入出力・agent step の対話履歴について新規の保存先・新規のログ出力・新規の露出経路（外部送信・追加永続化等）を追加しない。

**背景**: マイルストーン [02]–[06] により、workflow engine 側では `run_id` を主語に state 観測・state 変化を扱う土台が完成した。

- [02] で workflow / node / run / step / event の中核モデルが新 schema に揃った。
- [03] で `run_id` を一次キーとした run metadata と event log の永続化基盤が成立した（[issues-1011](./issues-1011.md)）。
- [04] で workflow state を変化させる入口が `WorkflowCommand` 型に typed 化され、Tauri command 入口が `run_id` 主語に揃った（[issues-1013](./issues-1013.md)）。
- [05] で外部 caller が `run_id` を主語に workflow run を観測する read-only 経路（API + CLI）が確立し、CLI は file-direct 構成を採用した（[issues-1015](./issues-1015.md)）。
- [06] で CLI から `run_id` を主語に approve / reject / abort を要求できる mutating 経路が追加され、CLI 経路と UI 経路は engine から見て同一 `dispatch_external` に合流するようになった（[issues-1019](./issues-1019.md)）。

一方、現状の UI は workflow step を「個別の chat session（chat tab）」として可視化しており、利用者は run の進行を追うために多数の chat tab を切り替える必要がある。run 全体の事実列（timeline）や step ごとの詳細を一画面で観測する UI は存在せず、`run_id` 主語の engine モデル（[02]–[06]）と UI 表現の粒度が一致していない。plan doc が「workflow step が自由対話 chat tab と同格に見えない」「多数の chat session を切り替えなくても run を inspect できる」を完了条件として置いている通り、UI 側でも `run_id` を主語にした第一級の表示モデルが必要となっている。

本 issue は、[02]–[06] で確立された run_id 主語の engine モデルを UI 側に投影するための表示モデルとして Workflow panel を導入し、これに伴って workflow step の chat session を tab bar から panel 内 transcript へ移し替える。これにより、後続マイルストーン（[08] OutputForm CLI / [15] Skill / [16] Main Agent Mediation）が Workflow panel を「人間オペレータの一次観測 surface」として前提できるようになる。

**前提**:

- マイルストーン [05] Read-Only Run APIs + CLI 完了済み（[issues-1015](./issues-1015.md)）。`workflow_runs/` 配下の run metadata / event log を file-direct に読める projection helper（`event_projection::reconstruct_state_from_events`）が存在し、Workflow panel の表示元データとして再利用できる。
- マイルストーン [06] Mutating CLI 完了済み（[issues-1019](./issues-1019.md)）。CLI 経路と UI 経路が `dispatch_external` で合流する境界が確立されており、Workflow panel から発行する approval / reject / abort も既存 Tauri command 経由で同じ engine 入口に到達できる。
- マイルストーン [04] Command / Event Boundary 完了済み（[issues-1013](./issues-1013.md)）。`WorkflowCommand` typed 入口と `WorkflowEvent` 語彙、`WorkflowEngine::dispatch_external` 単一入口、認可・冪等性・stale target 判定が engine 内に閉じている。
- マイルストーン [03] Run Store / Run ID 完了済み（[issues-1011](./issues-1011.md)）。`run_id` を一次キーとする run metadata / event log の永続化基盤が成立済み。
- 既存右パネル（`src/components/panels/ReviewPanel.tsx` 等）と既存 chat tab 機構（`AgentChatPanel/`）はそのままの構造で存在し、Workflow モードは「中央エリア（ViewToolbar）の表示モード切替」として既存 AgentChat と並列に追加できる。

## 振る舞い定義

```gherkin
Feature: Workflow Panel / Command Center

  Rule: 利用者は中央エリアから Workflow 観測モードへ切り替えられる
    Scenario: 利用者が Workflow モードを選ぶ
      Given 利用者が現 worktree を開いている
      When 利用者が中央エリアの表示モードを Workflow に切り替える
      Then 現 worktree に紐づく workflow run の状況が一つのパネル内に表示される

  Rule: 利用者は現 worktree の workflow run を別画面に遷移せず一覧できる
    Scenario: 利用者が active run と過去 run を確認する
      Given 利用者が Workflow panel を開いている
      Then 現 worktree で進行中の run と過去の run が同一パネル内で観測できる

  Rule: 利用者は run の事実列を経路に依存せず時系列で観測できる
    Scenario: 利用者が run の経緯を時系列で確認する
      Given 利用者が観測対象の run を選んでいる
      Then 当該 run で「いつ / どの node で / どんな事実が起きたか」が時系列で観測できる

  Rule: 利用者は timeline 上の任意の step の詳細を確認できる
    Scenario: 利用者が step を選んで詳細を見る
      Given 利用者が観測対象の run の timeline を見ている
      When 利用者が timeline 上の特定の step を選ぶ
      Then その step の入出力・遷移結果・所要時間が確認できる

  Rule: 利用者は agent step の対話履歴を Workflow panel 内で読める
    Scenario: 利用者が agent step の対話を確認する
      Given 利用者が timeline 上の agent step を選んでいる
      Then その step の対話履歴が Workflow panel 内で読める

  Rule: 利用者は Workflow panel から進行中 run の state を変化させられる
    Scenario: 利用者が approval 待ちの step を承認する
      Given 利用者が Workflow panel で approval 待ちの step を選んでいる
      When 利用者がその step を承認する
      Then 当該 step の承認が run の進行に反映される

    Scenario: 利用者が approval 待ちの step を却下する
      Given 利用者が Workflow panel で approval 待ちの step を選んでいる
      When 利用者がその step を却下する
      Then 当該 step の却下が run の進行に反映される

    Scenario: 利用者が進行中の run を abort する
      Given 利用者が Workflow panel で進行中の run を選んでいる
      When 利用者がその run を abort する
      Then 当該 run が abort された事実が run の進行に反映される

  Rule: workflow step として起動された対話は自由対話 chat tab と同格に表示されない
    Scenario: workflow step の対話は tab bar に並ばない
      Given workflow run が進行中であり、その run の中で agent step が起動している
      Then それらの step の対話は自由対話 chat tab と同格には tab bar に並ばない
      And それらの step の対話は Workflow panel の step conversation transcript から読める

  Rule: Workflow panel の観測対象は現 worktree に紐づく run に限定される
    Scenario: 他 worktree の run はこのパネルに現れない
      Given 利用者が現 worktree の Workflow panel を開いている
      Then 他 worktree に紐づく run はこのパネルでは観測されない
```

## アーキテクチャ概要

### 責務配置

- 中央エリア Layout 層（`src/screens/MainLayout.tsx` / `src/components/layout/ViewToolbar.tsx` 等）: 中央エリア上部の ViewToolbar 上に Workflow 表示モードを既存 AgentChat と並列の切替先として登録する責務を持つ / Workflow モード内の表示構造（timeline・step detail・chat view の同居レイアウト等）の決定はここで持たない。AgentChatProvider で AgentChatPanel と WorkflowView を同じ AgentChat state にひもづける責務を持つ。
- Workflow panel フロントエンド層（`src/components/panels/WorkflowView/` 配下と `src/components/panels/WorkflowPanel/` 配下）: 現 worktree に紐づく run 一覧・選択中 run の timeline・選択中 step の詳細・選択中 agent step の chat session（composer 付き）を「invoke 結果の表示用整形」として描画する責務を持つ / 事実列の整序・所要時間計算・approval 可否判定・stale 判定など run の意味解釈は一切持たない。
- 共有 chat view（`src/components/panels/AgentChatPanel/ChatSessionView.tsx`）: 単一 ChatSession に対する message stream + composer を描画する presentational コンポーネント。AgentChatPanel と WorkflowView の双方から再利用される。session 取得・state 更新・送信ロジックは持たず、props で受け取った session / handler を表示するだけ。
- フロントエンド Hooks / State 層（`src/hooks/useAgentChat.ts` ＋ `src/contexts/AgentChatContext.tsx`）: 現 worktree の AgentChat 全体 state（sessions / activeSession / viewedStepSession / streaming / activity / send / interrupt / permission / model / backend / per-session lookup）を MainLayout レベルで一度だけ生成し、AgentChatProvider 経由で AgentChatPanel と WorkflowView へ供給する。step session の本文反映は `loadStepSession(sessionId)` 経由で `viewedStepSession` slot に書き込み、send/interrupt 等は sessionId-explicit な API で操作する。バックエンドからの View 型を加工して別ドメインモデルへ再構築する責務は持たない。
- Tauri command 層（`src-tauri/src/workflow/commands.rs` 等）: [05] で確立した read-only 経路（`get_workflow_state` / `list_workflow_runs` / `get_workflow_execution_log` 等）と [04] / [06] で確立した mutating 経路（`approve_workflow_step` / `reject_workflow_step` / `abort_workflow`）と既存 chat session 取得経路（`get_session`）を Workflow panel の到達先として再利用する責務を持つ / UI 専用の新 file-direct 経路や engine bypass 経路を追加しない。step session の transcript 取得には新 Tauri command を追加せず、既存 session 取得経路に閉じる。
- Workflow engine（Rust）層（`src-tauri/src/workflow/` 配下）: `dispatch_external` の単一入口・`WorkflowEvent` 列の永続化・`event_projection::reconstruct_state_from_events` による state 復元・worktree 単位の認可境界をそのまま提供する責務を持つ / 本 issue では engine 内部に新しいビジネスルール（新 command / 新 event / 新 state）を導入しない。
- Agent chat session 層（既存 `AgentChatPanel` / セッションストア）: agent step に紐づく chat session の transcript を保持・提供する責務を持つ（既存責務を据え置く） / workflow step として起動された chat session を tab bar 上に自由対話 chat tab と同格に列挙する責務は失う（tab bar 側で除外し、Workflow panel の chat view 経路を正規閲覧/操作経路とする）。

### データ/通信フロー

- 表示モード切替: 利用者操作 → 中央 ViewToolbar の mode toggle → MainLayout の centerMode 状態 → Workflow panel コンポーネント（WorkflowView）が現 worktree path を prop として受けてマウント。
- run 一覧表示: Workflow panel マウント → 現 worktree 向け hook → `list_workflow_runs(worktree_path)` invoke → engine が worktree 認可境界で filter した run metadata 列を返す → UI 描画。
- timeline / step detail 表示: 利用者が run を選択 → `get_workflow_execution_log(run_id)` invoke → engine が `WorkflowEvent` 列と `reconstruct_state_from_events` 由来の復元 state を返す → UI は事実列と step 状態を時系列・選択中 step 詳細としてそのまま描画。step の入出力・所要時間等のメタ情報は `get_workflow_step_detail` invoke で取得する。
- step conversation 表示: 利用者が agent step を選択 → 復元 state / run metadata 内の `chat_session_id` を引き当て → AgentChatProvider 経由で共有された useAgentChat の `loadStepSession(sessionId)` を呼ぶ → 既存 `get_session` Tauri command で当該 session の完全 ChatSession を取得 → reducer の `viewedStepSession` slot に格納 → Workflow panel 内の ChatSessionView が描画。tab bar は開かず、AgentChatPanel 本体の表示にも影響しない（同じ session を二重表示しない）。送信は `sendMessage(stepSessionId, content, ...)`、interrupt / permission 応答 / モデル変更等も sessionId-explicit な API で実行され、engine 側の認可境界はすべて既存経路を通る。
- approval / reject / abort 発行: Workflow panel 上のボタン操作 → 既存 Tauri command（`approve_workflow_step` / `reject_workflow_step` / `abort_workflow`） invoke → engine の `dispatch_external` 単一入口に合流 → 発火した `WorkflowEvent` が event log と active state に反映 → UI は次回購読時に最新事実列を反映。
- 現 worktree スコープ強制: 全ての観測 / 操作 invoke で「現 worktree path」が必ず caller 側引数として明示される → engine 側で `canonicalize_managed_worktree_path` 経由の認可境界を通る。

### 状態Owner

- 現 worktree path: フロントエンド MainLayout / useWorktreeState（既存）。Workflow panel はこれを prop / context として受け取るだけで自身では保持しない。
- 中央エリアの現在表示モード（Agent / Workflow）: フロントエンド MainLayout（centerMode 状態、永続化しない）。
- 観測対象 run の選択・観測対象 step の選択・スクロール位置等の UI ローカル状態: Workflow panel コンポーネント内の React state。
- AgentChat 全体 state（sessions / activeSession / viewedStepSession / turnPhases / sessionAgentStates / streaming 中の messages 等）: AgentChatProvider にホストされた `useAgentChat` の reducer state。MainLayout 配下の AgentChatPanel と WorkflowView が Context 経由で共有する。
- workflow run の事実列（`WorkflowEvent` 履歴） / run metadata / 復元 state: workflow engine（Rust 側 run store + event log）。フロントエンドは購読のみで mutate しない。
- approval / reject / abort 等 run progression: workflow engine（`dispatch_external` 経路のみで遷移）。UI は要求の発行元であり、結果は engine 由来の事実列を介してのみ観測する。
- agent step に紐づく chat session の本文: 既存 agent chat session ストア（Rust 側）。Workflow panel は既存 `get_session` 取得経路を借りて `viewedStepSession` に投影するのみで、専用 transcript ストアや専用取得 Tauri command を持たない。

### 境界

- フロントエンドはロジックを持たない（`.claude/rules/rust-first-logic.md` 準拠）。timeline 整序、step 所要時間算出、approval 可否判定、stale step 判定等の意味解釈は engine / projection が返す事実 / View 型に閉じる。
- Workflow モードと既存 Review / Source Control / 自由対話 chat tab は並列の表示先であり、Workflow モードの追加によって既存表示モードの状態・経路は破壊しない（現状温存節の境界）。
- workflow step として起動された chat session は tab bar 上に自由対話 chat tab と同格に列挙しない。tab bar 側のフィルタ境界を Workflow run の事実（run metadata / step → chat_session_id 紐付け）に基づいて引く。
- Workflow panel から発行する approval / reject / abort は、必ず [04] / [06] で確立した typed command boundary（`WorkflowCommand::ApproveNode` / `RejectNode` / `AbortRun` → `dispatch_external`）に到達する。UI 専用の新 file-direct 経路や engine bypass 経路は本 issue で導入しない。
- 観測対象は現 worktree に紐づく run に限定する。Workflow panel から発行する全 invoke で worktree path を明示し、engine 側の worktree 認可境界（[05]）を必ず通す。他 worktree の run を横断観測する経路は本 issue では生やさない。
- agent step transcript の取得は既存 agent chat session の取得経路（`get_session` Tauri command）を再利用する境界とし、Workflow panel 側に「workflow step 専用の transcript 経路」を新設しない。transcript 取得時の `chat_session_id` は、選択中 run の step metadata（engine 側で発行・永続化された値）から引き当てる経路に限定し、フロントエンド側で任意の `chat_session_id` を組み立てる経路は持たない。送信・interrupt・permission 応答等の operations も AgentChat の共有 reducer 経由で既存 chat 経路の Tauri command にそのまま到達するため、新規 file-direct 経路や engine bypass 経路は本 issue では生やさない。
- 本 issue が Workflow panel に表示する `WorkflowEvent` payload・step 入出力・transcript 本文は、ファイル / CLI / agent 出力 / ユーザー入力に由来し得る untrusted data として扱う。UI 描画時は React の標準エスケープに頼り、`dangerouslySetInnerHTML` 等の生 HTML 注入経路を本 issue では追加しない。`run_id` / `step_id` / `chat_session_id` 等の識別子はフロントエンドで合成せず、engine 側で発行・検証された値（read-only API / event payload / run metadata 由来）のみを invoke 引数として用いる。

### 実装に委ねること

- Workflow panel 内のサブコンポーネント分割（run 一覧 / run サマリ / timeline / step detail / transcript 等を別コンポーネントに切り出す粒度）。
- timeline の視覚的表現形式（縦タイムライン・リスト・グルーピング等）、step バッジ・アイコン・配色の選択。
- 「現 worktree の run 一覧」が空 / 履歴のみ / active あり等の空状態文言と微細レイアウト。
- フロントエンド hook の分割粒度（run 一覧購読・run detail 購読・transcript 購読を別 hook にするか統合するか等）。
- TypeScript 側ローカル型エイリアスや View 型からの薄い派生型の命名（既存 `protocol/workflow.rs` の View 型を一次ソースとする前提で）。
- run / step / event 一覧表示用に Rust 側 view helper を追加する必要が出た場合、その helper 関数名・モジュール内配置（既存 `protocol/workflow.rs` / `event_projection` の責務境界に従う形で）。
- テストファイルの具体的配置と Gherkin シナリオ単位での自己検証テストケースの命名・分割。
- workflow step を tab bar から除外するためのフィルタ実装上の具体的箇所（既存の workflow session 区別経路の延長として取るか、新フィルタを差すか）。

## 関連マイルストーン上の位置

- 直接の依存元: [03] Run Store / Run ID / [04] Command / Event Boundary / [05] Read-Only Run APIs + CLI / [06] Mutating CLI。
- 直接の依存先: [08] OutputForm CLI / [15] Skill / [16] Main Agent Mediation。
- 本 issue は「engine から見て run_id 主語に揃った世界」を UI 表現に投影する範囲に閉じる。新規 run 起動 / structured output 提出 UI / bash node 実行 / agent narrator はそれぞれ別マイルストーンに委ねる。
