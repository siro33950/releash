# issues-1013: Workflow Engine Evolution - Command / Event Boundary

関連: [GitHub Issue #1013](https://github.com/siro33950/releash/issues/1013) / マイルストーン [04] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) / 先行 [issues-1011](./issues-1011.md)

## 要求

**種別**: 新機能

**ゴール**: workflow engine の state 変化を typed な `WorkflowCommand` 経由に一本化し、engine が発行する事実列を typed な `WorkflowEvent` に一本化する。具体的には以下が満たされること。

- workflow state を変化させる入口として `WorkflowCommand` 型が存在し、`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode` の 4 つの command が typed に表現される。
- 上記 4 つの command に対する command handler が engine 側に実装され、UI / Tauri command / 内部呼び出し元から typed command を経由して engine に到達する経路が成立する。
- 既存 Tauri command（workflow 操作系：起動 / 中断 / 承認 / 却下）は、入口形を破壊的に変更してよく、内部で `WorkflowCommand` を組み立てて engine に渡す形に置換される。後方互換 wrapper は要求しない。
- engine が発行する append-only な事実列を `WorkflowEvent` 型に一本化する。本 issue では `WorkflowEvent` 語彙の確立と、本 issue 対象 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）に対応する event 発行点の `WorkflowEvent` への置換を範囲とする。既存 `WorkflowLogEvent` 列挙体・NDJSON event log は新語彙へ完全置換し、旧 NDJSON 在庫は破棄前提とする。`CompleteNode` / `FailNode` 等の engine 内部発行点の typed 化は本 issue では対象外（[05] へ振り分け）。
- 上記 4 command の handler および対応する WorkflowEvent 発行に対して内部 path のテストが存在する。

**スコープ外**: 以下は後続マイルストーンに明示的に振り分け済み。

- read-only run APIs / CLI（[05]）: `list_workflow_runs` / `get_workflow_run` / `get_workflow_run_log` / `get_workflow_run_state` 等。
- **engine 内部 typed 遷移 command の `WorkflowCommand::CompleteNode` / `FailNode` の導入（[05] に追記）**: 外部入口を持たない engine 内部用途中心 command。NodeCompleted / NodeFailed event 発行点の typed 化として [05] の観測経路整備と同マイルストーンに揃える。本振り分けに伴い `docs/workflow-engine-evolution-plan.md` [05] 節と `docs/workflow-engine-model-boundary.md` の該当箇所を本 issue の作業で更新済み。
- mutating CLI（[06]）: `releash workflow approve|reject|abort` 等の CLI 入口。本 issue は内部 command 層までを対象とし、CLI バイナリは含めない。
- Workflow Panel / Command Center UI（[07]）。
- structured output 提出（[08]）: `WorkflowCommand::SubmitOutput` および `OutputForm CLI`。本 issue で型として導入することはしない。
- bash node 実行系統（[13]）。
- main-agent narrator への typed event 配信と user decision の CLI/API 戻り経路整備（[16]）。

**現状温存**: agent 返信テキスト解釈に基づく engine 内部の分岐ロジック（aggregate の LGTM マッチング、`<workflow_output>` 抽出など step agent 出力解釈経路）は本 issue では廃止対象としない。plan doc [08] L437 で `<workflow_output>` 抽出は fallback として維持する方針が明記されており、これらは廃止ではなく現状の振る舞いを維持する。[04] 完了条件「state transition が main-agent free text に依存しない」の "main-agent" は user-facing narrator を指し、step agent 出力解釈とは別軸である。本 issue では user decision 系の 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）を typed 化することで完了条件を満たす。

**背景**: マイルストーン [03] までで workflow 実行は `run_id` を主語として参照できるようになり、API・engine 内部キー・persistence のすべてが `run_id` を一次キーとする形に揃った（[issues-1011](./issues-1011.md) 参照）。一方、state を変化させる入口は依然として個別の Tauri command の引数列として散在しており、authorization / 冪等性 / stale target 判定 / 経路非依存性（UI から呼ばれても CLI から呼ばれても等価に扱う）といった共通検査の起点が存在しない。今後の [05] 以降の read-only API / [06] mutating CLI / [07] Workflow Panel / [16] Main Agent Mediation はいずれも「state 変化は同一の typed command を経由する」「engine の事実は同一の typed event 語彙で観測する」ことを前提とする。本 issue はその土台として、Command / Event の boundary を確立する。

**前提**: マイルストーン [03] Run Store / Run ID 完了済み。`run_id` 主語の API・engine 内部キー・`workflow_runs/{run_id}.json` 永続化はすでに成立している。

## 関連マイルストーン上の位置

- 直接の依存元: [02] Normalized Workflow / [03] Run Store。
- 直接の依存先: [05] Read-Only Run APIs + CLI / [06] Mutating CLI / [07] Workflow Panel / [16] Main Agent Mediation。
- 本 issue は内部 boundary の確立に閉じ、CLI バイナリ・UI パネル・外部観測経路は後続に委ねる。

## 振る舞い定義

```gherkin
Feature: Workflow Engine の Command / Event Boundary

  Rule: workflow の state は typed command の受理によってのみ変化する
    Scenario: ユーザーが workflow run の起動を要求する
      Given ユーザーが起動可能な workflow を選択している
      And ユーザーがその workflow を起動できる認可済み主体である
      When ユーザーが run の起動を要求する
      Then 新しい run が開始される

    Scenario: ユーザーが進行中の run の中断を要求する
      Given 進行中の workflow run が存在する
      And ユーザーがその run を操作できる認可済み主体である
      When ユーザーがその run の中断を要求する
      Then その run が中断される

    Scenario: ユーザーが承認待ち node の承認を要求する
      Given 承認待ちの workflow node が存在する
      And ユーザーがその node を判断できる認可済み主体である
      When ユーザーがその node の承認を要求する
      Then その node が承認された判断として受理される

    Scenario: ユーザーが承認待ち node の却下を要求する
      Given 承認待ちの workflow node が存在する
      And ユーザーがその node を判断できる認可済み主体である
      When ユーザーがその node の却下を要求する
      Then その node が却下された判断として受理される

  Rule: 同一意図の command は呼び出し経路に依らず等価に扱われる
    Scenario: 同じ意図の command が異なる入口から発行される
      Given ユーザーが同じ意図を異なる入口から表明できる
      When ユーザーがいずれかの入口から意図を表明する
      Then どの入口を経由しても workflow engine は同じ振る舞いを示す

  Rule: 権限の無い / 対象不在 / 既決の command は state 変化を起こさない
    Scenario: 認可されていない主体が command を要求する
      Given 対象 run / node を操作する権限を持たない主体である
      When その主体が対象 run / node に対する command を要求する
      Then workflow engine の state は変化せず、その command は受理されない

    Scenario: 存在しない run / node を対象にした command が要求される
      Given 対象として指定された run / node が workflow engine に存在しない
      When その存在しない run / node に対する command が要求される
      Then workflow engine の state は変化せず、その command は受理されない

    Scenario: 既に終了した run に対する操作 command が要求される
      Given 対象の workflow run は既に終了している
      When その run に対する中断 command が要求される
      Then workflow engine の state は変化せず、その command は受理されない

    Scenario: 既に判断済みの node に対して同一意図の command が再度要求される
      Given 対象の workflow node に対する承認 / 却下の判断は既に受理されている
      When 同一意図の判断 command が再度要求される
      Then 2 度目以降の command は state を変化させず、最初の判断が維持される

  Rule: 本 issue が typed 化する経路の state 変化は typed event の append-only な列として記録される
    Scenario: 本 issue 対象 command の受理によって state 変化が発生する
      Given workflow engine が稼働している
      When 本 issue 対象 command（起動 / 中断 / 承認 / 却下）の受理によって state 変化が起こる
      Then 対応する事実が typed event として末尾に追記される

    Scenario: 観測者が workflow の進行を辿る
      Given 過去に発生した workflow の事実列が記録されている
      When 観測者がその run の事実列を参照する
      Then 観測者は統一された event 語彙で進行を辿れる

  Rule: command 受理サイクル内で event append / 永続化が失敗した場合、engine state は command 受理前の状態に完全復元される
    Scenario: state mutation 後の event append が失敗する
      Given 本 issue 対象 command が受理されようとしている
      When command handler 内で state mutation を行った後に event append または永続化が失敗する
      Then engine state（履歴・変数・current_step_index 等を含む WorkflowExecution 全体）は mutation 直前の状態に完全に戻され、追加副作用は一切実行されない

    Scenario: 部分復元は許容されない
      Given command handler が mutation 直前の WorkflowExecution snapshot を保持している
      When mutation 後の event append / Run Store sync / 永続化のいずれかが失敗する
      Then handler は保持している snapshot で WorkflowExecution 全体を一括復元する（特定フィールドのみを戻す部分 rollback は行わない）
```

## アーキテクチャ概要

本 issue は「state を変化させる入口の typed 化」と「engine が発行する事実列の typed 化」を、内部 boundary として確立する。CLI バイナリ・UI パネル・engine 内部の `CompleteNode` / `FailNode` typed 化は後続マイルストーンに委ねる（前掲スコープ外参照）。

### 責務配置

- **`workflow/command.rs`（新設）**: `WorkflowCommand` 型と受理結果型（`WorkflowCommandResult`）を所有する。本 issue では `StartRun` / `AbortRun` / `ApproveNode` / `RejectNode` の 4 variant を typed に表現することのみを担い、ハンドラ実体は持たない。`SubmitOutput` / `CompleteNode` / `FailNode` は本 issue では導入しない。**sentinel 禁止**: `WorkflowCommandResult` から特定 variant への変換ヘルパー（特に `Accepted` を空文字列 `run_id` に sentinel 化する変換）を本ファイルでは提供しない。typed 境界からの薄い変換（`Result<String, _>` / `Result<(), _>` への射影）は呼び出し側 Tauri adapter（`commands.rs`）が `match` で variant 別に行い、本来到達しない variant は内部不整合として `Err` に変換する。
- **`workflow/event.rs`（新設）**: `WorkflowEvent` 型を所有する。append-only な事実列の型と NDJSON 表現を担う。`WorkflowLogEvent` 由来の語彙を新 vocabulary（`RunStarted` / `NodeStarted` / `NodeCompleted` / `ApprovalRequested` / `ApprovalResolved` / `RunCompleted` / `RunFailed` / `RunAborted` 等）に完全置換する形で再定義する。旧 NDJSON 在庫を読む責務は担わない。
- **`workflow/engine.rs`（future core / 既存）**: 4 command に対する command handler を engine 側の入口として実装する。既存の `start_workflow` / `abort_workflow_by_run_id` / `handle_approval` は command handler に統合される（method の再編は実装に委ねる）。state 遷移の権威であり続け、`WorkflowEvent` 発行点を engine 内部の単一経路に集約する（重複発行禁止）。state transition は agent free text に依存しない（user decision 4 command の範囲）。**rollback owner**: command 受理サイクル内で event append / 永続化失敗時の atomic rollback は本ファイルが owner となる。各 command handler は mutation 直前の `WorkflowExecution` 全体の snapshot を保持し、append / sync / persist のいずれかが失敗した場合に `*exec = snapshot_before;` の形で `WorkflowExecution` 全フィールド（履歴・変数・state・current_step_index 等）を一括復元する。部分復元用 helper は導入しない。
- **`workflow/commands.rs`（compat adapter / 既存）**: 既存 Tauri command（`start_workflow` / `abort_workflow` / `approve_workflow_step` および却下相当）は、内部で `WorkflowCommand` を組み立てて engine の command 入口に渡す薄い変換層に置換される。入口形は破壊的に変更してよく、後方互換 wrapper は導入しない。
- **`workflow/log.rs`（既存）**: 旧 `WorkflowLogEvent` 列挙体・在庫互換コードは削除し、`WorkflowEvent` を NDJSON で append する書き込み機構の責務に縮退する。旧 NDJSON 在庫は破棄前提で扱う。
- **`workflow/run.rs`（既存 / Run Store）**: run lifecycle（`WorkflowRun` / `RunStatus`）の所有責務は変えない。状態更新は command handler 経由でのみ呼ばれる。本 issue で schema は変更しない。
- **フロントエンド（`src/`）**: 既存の invoke 呼び出し点を typed command 引数に合わせて更新する以外、ロジックは追加しない。`rust-first-logic.md` を厳守する。

### データ/通信フロー

- **起動**: UI / Remote → Tauri command（`start_workflow`） → `WorkflowCommand::StartRun` 組み立て → engine command handler → run 開始 → `WorkflowEvent::RunStarted` を発行（log.rs 経由で NDJSON 追記） → UI 同期。
- **中断**: UI / Remote → Tauri command（`abort_workflow`） → `WorkflowCommand::AbortRun { run_id, .. }` → engine command handler → run 終了 → `WorkflowEvent::RunAborted` 発行。
- **承認 / 却下**: UI / Remote → Tauri command（`approve_workflow_step` および却下相当） → `WorkflowCommand::ApproveNode` / `RejectNode` → engine command handler → approval 解決 → `WorkflowEvent::ApprovalResolved`（必要に応じて後続 node の `NodeStarted` 等）を発行。
- **観測**: 本 issue 対象 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）経由の state 変化に対応する event 発行は、engine 内の単一発行点を経由し、`WorkflowEvent` として NDJSON に append される。`CompleteNode` / `FailNode` 等の本 issue スコープ外の内部発行点は対象外（前掲スコープ外参照）。observer は run_id を主語に event 列を辿れる。

### 状態Owner

- **`WorkflowCommand`（in-flight）**: 値オブジェクト。所有者なし（呼び出しのたびにスタック上で生成）。永続化しない。
- **command 検査結果（authorization / 冪等性 / stale target 判定）**: engine 内の command handler。検査ロジックの実体は engine（`engine.rs`）に置く。
- **`WorkflowEvent` 列（過去事実）**: `log.rs` 経由で NDJSON ファイルが一次 owner。発行点は engine の単一経路。
- **`WorkflowRun` lifecycle（現在状態）**: `run.rs` の Run Store（active map + `workflow_runs/{run_id}.json`）。command handler の結果として更新される。
- **`WorkflowState` / `StepHistoryEntry` 等の既存 UI 投影**: 引き続き `state.rs` が owner（本 issue では shape を変えない）。

### 境界

- **外部入口 → engine**: Tauri command・内部呼び出し元は engine の private method を直接叩かず、必ず `WorkflowCommand` を経由する。新規入口（CLI / agent 等）も同じ境界に揃える前提を本 issue で確立する。
- **engine → 観測者**: state 遷移の事実は `WorkflowEvent` の append-only 列としてのみ外部化される。発行済み event は書き換わらない。撤回・補正も追加 event として表現する。
- **新旧境界**: 旧 `WorkflowLogEvent` 語彙・旧 NDJSON 在庫は本 issue で廃止する。互換 wrapper は導入しない。
- **scope の境界**: 本 issue は 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）と対応する event 発行点に閉じる。`CompleteNode` / `FailNode` / `SubmitOutput` / read-only API / CLI バイナリ / Workflow Panel UI は導入しない。
- **agent 返信解釈の温存境界**: aggregate の LGTM マッチング・`<workflow_output>` 抽出など step agent 出力解釈経路は本 issue では削除も typed 化もしない。
- **atomic mutation 境界**: command 受理サイクル（command 受理 → state mutation → event append → 永続化 → 副作用）は WorkflowExecution に対する atomic な単位として扱う。mutation 後の append / 永続化失敗時には、mutation 直前の `WorkflowExecution` snapshot に**完全復元**する。state / current_step_index のみを戻す部分 rollback は禁止する（履歴や変数の更新が残ったまま required event が欠落する不整合を構造的に排除する）。
- **テスト境界**: 本 issue 対象 4 command の routing / 受理判定 / event append / 拒否時 no-append / 受理時 state mutation の検証は、production の `WorkflowEngine::dispatch` を直接呼ぶ harness 経由で行う。`AppHandle` 依存は `tauri::test`（`mock_builder` / `mock_app`）で構築した実 `AppHandle` で満たす。production dispatch と別に match 分岐を再実装する test-only の並行 dispatcher（影 dispatcher）の導入は禁止する。

### 実装に委ねること

- `WorkflowCommand` / `WorkflowEvent` の各 variant の具体的なフィールド shape（型名・field 名・`Option` 化の粒度）。
- command dispatcher のシグネチャ（一括 `dispatch(cmd: WorkflowCommand)` か variant 別 `handle_*` か）。
- engine 内既存メソッド（`start_workflow` / `abort_workflow_by_run_id` / `handle_approval`）の改名・分割・private 化の粒度。
- 旧 `WorkflowLogEvent` variant から新 `WorkflowEvent` variant へのマッピング切り分け（特に `OutputCollected` / `ParallelStarted` / `ParallelStepStarted` 等の置換語彙）と、`NodeStarted` / `NodeCompleted` で parallel 子 node をどう表現するかの具体形。
- 既存 Tauri command の引数 typed 化の粒度（呼び出し側互換は不要だが、フロント側 invoke 引数の最小変更は実装に委ねる）。
- command handler 内の認可・冪等性・stale target 判定の具体的判定条件（既存 engine 振る舞いと等価であれば実装判断でよい）。
- 新規モジュール（`command.rs` / `event.rs`）の内部分割（同一ファイル内 enum か、サブモジュール化か）。
- `WorkflowEvent` NDJSON のファイル配置・ファイル名規約（既存 `WorkflowEventLog` 設計の踏襲可否を含む）。
