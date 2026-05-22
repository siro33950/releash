# issues-1015: Workflow Engine Evolution - Read-Only Run APIs + CLI

関連: [GitHub Issue #1015](https://github.com/siro33950/releash/issues/1015) / マイルストーン [05] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) / 先行 [issues-1011](./issues-1011.md) / [issues-1013](./issues-1013.md)

## 要求

**種別**: 新機能

**ゴール**: 外部 caller（UI / Remote / Agent / 人間オペレータ）が `run_id` を主語として workflow run を観測でき、engine 内部の node 完了 / 失敗遷移も typed command として揃った状態を作る。具体的には以下が満たされること。

- 外部 caller が workflow template の一覧と過去 / 進行中の workflow run を `run_id` 主語で観測できる read-only な API 入口（Tauri / local API command）が成立している。
  - workflow template の一覧を取得できる（`list_workflows`）。
  - 過去および進行中の workflow run の一覧を取得できる（`list_workflow_runs`）。
  - 単一の run の summary metadata を取得できる（`get_workflow_run`）。
  - 単一の run の event log を取得できる（`get_workflow_run_log`）。
  - 単一の run の現在 state を取得できる（`get_workflow_run_state`）。
- 上記 API と等価な観測手段を CLI からも実行できる。
  - `releash workflow list` — 利用可能な workflow template の一覧。
  - `releash workflow runs` — 過去 / 進行中の run の一覧。
  - `releash workflow status <run-id>` — 指定 run の現在 state。
  - `releash workflow logs <run-id>` — 指定 run の event log。
- engine 内部の node 完了 / 失敗遷移が typed command（`WorkflowCommand::CompleteNode` / `FailNode`）経由で行われる。
  - これらは外部入口（UI / CLI / Agent）には公開せず、engine 内部の状態遷移を typed に表現するための command である。
  - 対応する `WorkflowEvent::NodeCompleted` / `NodeFailed` の発行点が typed command の経路に集約される。
- read-only API / CLI 経由で取得できる情報は、engine が一次 owner として保持する run metadata / event log / state の投影であり、別系統のキャッシュや再構築は行わない。
- running workflow を `run_id` で inspect でき、completed workflow の log を `run_id` で読める。

**スコープ外**: 以下は後続マイルストーンに明示的に振り分け済み。

- mutating CLI（[06]）: `releash workflow run` / `approve` / `reject` / `abort` 等の CLI 入口。本 issue は read-only 観測経路と engine 内部の node 完了 / 失敗 typed 化に閉じる。
- structured output 提出 CLI（[08]）: `WorkflowCommand::SubmitOutput` / `releash workflow output submit|validate|get`。
- Workflow Panel / Command Center UI（[07]）: 本 issue では UI パネル新規追加は行わない。既存 UI が API を消費する経路の最小調整は許容するが、新パネル導入は対象外。
- bash node 実行系統（[13]）: `releash workflow status` の表示対象に `bash` node が含まれても、bash 実行ランタイム自体の整備は行わない。
- main-agent narrator への typed event 配信と user decision の CLI 戻り経路整備（[16]）。
- engine 内部 typed command の他 variant（`SubmitOutput` 等、本 issue 対象外）の導入。

**現状温存**: agent 返信テキスト解釈に基づく engine 内部の分岐ロジック（aggregate の LGTM マッチング、`<workflow_output>` 抽出など step agent 出力解釈経路）は本 issue では廃止対象としない（[issues-1013](./issues-1013.md) と同じ方針を踏襲）。

**背景**: マイルストーン [04] までで workflow state を変化させる入口は `WorkflowCommand` 型に typed 化され、engine が発行する事実列は `WorkflowEvent` 語彙に集約された（[issues-1013](./issues-1013.md)）。一方、外部 caller が `run_id` を主語に workflow run を観測する手段は依然として既存 worktree-scoped Tauri command の戻り値に散在しており、CLI 入口は存在しない。また、user decision 系 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）は [04] で typed 化されたが、engine 内部の node 完了 / 失敗遷移は依然として method 直接呼び出しとして表現されており、typed boundary の片側だけが欠けている状態にある。

本 issue は、後続の [06] mutating CLI / [07] Workflow Panel / [16] Main Agent Mediation がいずれも「`run_id` を主語に観測でき、engine 内部の全 state 遷移が typed command 経由である」ことを前提とするため、その土台として read-only 観測経路と engine 内部 typed 化の残務を確立する。

**前提**: マイルストーン [04] Command / Event Boundary 完了済み（[issues-1013](./issues-1013.md)）。`WorkflowCommand` typed 入口 4 variant（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）と `WorkflowEvent` 語彙は確立済み。Run Store（[03] / [issues-1011](./issues-1011.md)）により `run_id` を一次キーとする run metadata / event log の永続化基盤は成立済み。

## 関連マイルストーン上の位置

- 直接の依存元: [03] Run Store / Run ID / [04] Command / Event Boundary。
- 直接の依存先: [06] Mutating CLI / [07] Workflow Panel / [16] Main Agent Mediation。
- 本 issue は read-only 観測経路と engine 内部 typed 化の残務に閉じ、mutating CLI / UI パネル / structured output は後続に委ねる。

## 振る舞い定義

```gherkin
Feature: Workflow Run の観測と engine 内部状態遷移の typed boundary

  Rule: 外部 caller は run_id を主語として workflow run を観測できる
    Scenario: 利用可能な workflow template の一覧を観測する
      Given workflow engine が利用可能な workflow template を保持している
      When 外部 caller が workflow template の一覧を要求する
      Then 利用可能な workflow template の一覧が外部 caller に伝わる

    Scenario: workflow run の一覧を観測する
      Given workflow engine が過去または進行中の workflow run を保持している
      When 外部 caller が workflow run の一覧を要求する
      Then 過去および進行中の workflow run の一覧が外部 caller に伝わる

    Scenario: 指定 run の summary metadata を観測する
      Given workflow engine が指定の run_id に対応する workflow run を保持している
      When 外部 caller が当該 run の summary metadata を要求する
      Then 当該 run の summary metadata が外部 caller に伝わる

    Scenario: 指定 run の event log を観測する
      Given workflow engine が指定の run_id に対応する workflow run を保持している
      When 外部 caller が当該 run の event log を要求する
      Then 当該 run の event log が外部 caller に伝わる

    Scenario: 指定 run の現在 state を観測する
      Given workflow engine が指定の run_id に対応する workflow run を保持している
      When 外部 caller が当該 run の現在 state を要求する
      Then 当該 run の現在 state が外部 caller に伝わる

    Scenario: 進行中の run を run_id で inspect する
      Given workflow engine 上で workflow run が進行中である
      When 外部 caller が当該 run を run_id で inspect する
      Then 進行中の run の現在 state が外部 caller に伝わる

    Scenario: 完了済みの run の log を run_id で読む
      Given workflow engine 上で workflow run が完了している
      When 外部 caller が当該 run の event log を run_id で要求する
      Then 完了済みの run の event log が外部 caller に伝わる

  Rule: 観測経路は API と CLI で等価な手段を提供する
    Scenario: API 経路と CLI 経路で同等の観測結果が得られる
      Given workflow engine が観測対象の workflow template / run / state / event log を保持している
      When 外部 caller が API 経路または CLI 経路を通じて同一の観測項目を要求する
      Then いずれの経路からも同等の観測結果が外部 caller に伝わる

  Rule: 観測される情報は engine が一次 owner として保持するデータと一致する
    Scenario: read-only な観測結果は engine 保持の情報と一致する
      Given workflow engine が run metadata / event log / state を一次 owner として保持している
      When 外部 caller が読み取り経路を通じてこれらを観測する
      Then 観測結果は engine が保持している run metadata / event log / state と一致する形で外部 caller に伝わる

  Rule: 観測経路は権限を持つ caller のみに開かれる
    Scenario: 権限を持たない caller は workflow run を観測できない
      Given workflow engine が workflow template / run / state / event log を保持している
      When 観測権限を持たない caller が API / CLI / Remote 経路で観測を要求する
      Then 観測結果は当該 caller には伝わらない

    Scenario: 権限を持つ caller は workflow run を観測できる
      Given workflow engine が workflow template / run / state / event log を保持している
      When 観測権限を持つ caller が API / CLI / Remote 経路で観測を要求する
      Then 観測結果が当該 caller に伝わる

  Rule: 観測対象として存在しない run_id は明示的に「該当 run なし」として扱われる
    Scenario: 存在しない run_id を観測しようとする
      Given workflow engine が指定された run_id に対応する workflow run を保持していない
      When 外部 caller が当該 run_id で summary metadata / event log / 現在 state を要求する
      Then 「該当 run なし」が外部 caller に伝わる

  Rule: 外部 caller には node 完了 / 失敗を直接要求する操作が提供されない
    Scenario: node が完了したときの状態遷移が run に反映される
      Given workflow run の中で node が処理されている
      When その node が完了する
      Then node 完了が workflow run の状態に反映され、node 完了の事実が event log に記録される

    Scenario: node が失敗したときの状態遷移が run に反映される
      Given workflow run の中で node が処理されている
      When その node が失敗する
      Then node 失敗が workflow run の状態に反映され、node 失敗の事実が event log に記録される

    Scenario: 外部 caller から node 完了 / 失敗を直接要求できない
      Given workflow engine が node 完了 / 失敗の状態遷移を内部的に処理している
      When 外部 caller が UI / CLI / Agent 経路を通じて利用可能な操作を観測する
      Then 外部 caller には node 完了 / 失敗を直接要求できる操作は提供されない
```

## アーキテクチャ概要

本 issue は (a) `run_id` 主語の read-only 観測経路（Tauri API / CLI）を確立すること、(b) engine 内部の node 完了 / 失敗遷移を typed command 経路に集約すること、の 2 系統を内部 boundary として確立する。observation source-of-truth は **engine が一次 owner として書き出すファイル群**（`workflow_runs/{run_id}.json` と NDJSON event log）に揃え、CLI は Archon 事例（[Archon CLI Reference](https://archon.diy/reference/cli/)）に倣い **engine と IPC せず file-direct でこれらを読む** 構成とする。in-memory active run map と persistence は state 遷移と同期する atomic mutation 境界の中で揃える（[04] 既存境界に整合）。

### 責務配置

- **`workflow/command.rs`（既存・拡張）**: `WorkflowCommand` enum に internal variant として `CompleteNode` / `FailNode` を追加する。外部入口（Tauri adapter / CLI / agent path）からこれら variant を組み立てる経路は提供しない。担当しない: handler 実体、CLI / Tauri adapter からの組み立て。
- **`workflow/engine.rs`（既存・拡張）**: 既存 `WorkflowEngine::dispatch` を内部 node 完了 / 失敗の発火点とし、現状 method 直接呼び出しになっている経路を `WorkflowCommand::CompleteNode` / `FailNode` を組み立てて `dispatch` に渡す形に集約する。`WorkflowEvent::NodeCompleted` / `NodeFailed` の発行点を typed command 経路の単一経路に揃える。rollback / atomic mutation 境界は [04] 既存境界をそのまま継承する。担当しない: 発行 event vocabulary の拡張。
- **`workflow/event.rs`（既存）**: `WorkflowEvent::NodeCompleted` / `NodeFailed` の shape は維持し、発行点が typed command 経由に変わるだけ。担当しない: 新規 variant 追加。
- **`workflow/commands.rs`（既存・破壊的置換）**: read-only Tauri command を spec 名称へ完全置換する。既存の `list_active_workflow_runs` / `list_completed_workflow_runs` / `list_workflow_runs_for_worktree` / `get_workflow_execution_log` / `get_workflow_execution_state` は **削除** し、以下に置き換える。後方互換 wrapper は導入しない。
  - `list_workflows` — workflow template 一覧（既存）
  - `list_workflow_runs` — active / terminal を含む run 一覧（active/completed 分離 command を統合）
  - `get_workflow_run` — 単一 run の summary metadata
  - `get_workflow_run_log` — `WorkflowEvent` 列
  - `get_workflow_run_state` — projection された run state view
- **`workflow/run.rs` / `workflow/log.rs` / `workflow/event_projection.rs`（既存）**: 観測経路の一次 source-of-truth として保持する。Tauri command も CLI も最終的にこれらの helper を介してファイルを読む。担当しない: 別系統の index / cache の導入。
- **CLI binary（新設）**: `releash workflow list / runs / status <run-id> / logs <run-id>` を提供する。**engine と IPC せず、`workflow_runs/` 配下と `workflows/` YAML / builtin を直接読む**。Tauri command と同じ projection helper を共有して divergence を防ぐ。担当しない: mutating 操作（[06] へ振り分け）、structured output 提出（[08]）、daemon 接続。
- **フロントエンド（`src/`）**: spec 名称の read-only API への invoke 置換と、戻り値の表示整形のみ。run 観測ロジックを持たない。

### データ/通信フロー

- **template 一覧（API）**: UI → `list_workflows` → `storage::list_workflows` + builtin → `Vec<Summary>` 返却。
- **template 一覧（CLI）**: `releash workflow list` → CLI が `workflows/` / builtin を直接読む → 同等の Summary 投影。
- **run 一覧（API）**: UI → `list_workflow_runs` → engine 経由で in-memory active map + `workflow_runs/{run_id}.json` 走査 → `Vec<WorkflowRunSummary>`。
- **run 一覧（CLI）**: `releash workflow runs` → CLI が `workflow_runs/` 配下を直接走査 → 同じ projection helper → 等価結果。
- **single run summary**: 観測者 → `get_workflow_run(run_id)` → in-memory または metadata file → `WorkflowRunSummary`。
- **run state**: 観測者 → `get_workflow_run_state(run_id)` または `releash workflow status <run-id>` → NDJSON 読込 → `event_projection::reconstruct_state_from_events` → state view。
- **run event log**: 観測者 → `get_workflow_run_log(run_id)` または `releash workflow logs <run-id>` → NDJSON 読込 → `Vec<WorkflowEvent>`。
- **node 完了 / 失敗（internal command）**: engine 内部 node 実行点 → `WorkflowCommand::CompleteNode { ... }` / `FailNode { ... }` を組み立て → `WorkflowEngine::dispatch` → state mutation + `WorkflowEvent::NodeCompleted` / `NodeFailed` 発行 → atomic rollback 境界は [04] 既存境界を継承。

### 状態 Owner

- **workflow template 集合**: `workflows/` YAML + builtin。read-only。観測経路は file-direct（Tauri / CLI ともに）。
- **active run 集合**: Run Store の in-memory map + `workflow_runs/{run_id}.json`（state 遷移ごとに engine が同期書き込み）。CLI は file 側のみを観測対象とする。
- **`WorkflowEvent` 列**: NDJSON ファイル（[04] 確立）。read-only API も CLI もここを唯一の log source-of-truth として扱う。
- **run state の投影**: `event_projection.rs` の `reconstruct_state_from_events`（既存）。NDJSON からの純粋関数 projection で、別系統のキャッシュは持たない。
- **internal node complete/fail の判断権威**: engine（既存）。typed command 経由化で発行点を集約するだけで権威は変わらない。

### 境界

- **read-only と mutating の分離**: 本 issue で導入する API / CLI 経路はすべて read-only。観測ルートからは engine の state mutation を起動しない。mutating CLI は [06] へ。
- **observation source-of-truth の境界**: read-only API も CLI も「engine が一次 owner として書き出した metadata / NDJSON」のみを観測対象とする。別系統のキャッシュ / index / 再構築（並行 reconstruct パス、別 DB、別 file format）は導入しない。
- **API / CLI の意味的等価性境界**: 同一 run_id / template に対し、API と CLI は同等の観測結果を返す。実装経路は分かれてよいが（Tauri command via engine vs. CLI による file-direct）、同じ projection helper を経由させて divergence を防ぐ。
- **CLI の認証境界**: CLI は同一デバイス所有者の OS user 権限下で動作する前提とし、`workflow_runs/` および `workflows/` の OS ファイル権限に依拠する。CLI 用の追加認証層は導入しない。リモートセッション経由の CLI は本 issue 対象外。
- **観測経路の認可境界**: 観測経路ごとに認可主体を分担する。Tauri API は既存 worktree-scoped Tauri 認証に依拠し、当該 worktree への操作権限を持つ UI セッションのみが観測できる。Remote 経路は既存 Remote 認可（HMAC トークン + セッション管理）の枠内に閉じ、Remote 認可を通過したセッションのみが観測できる。Agent 経路は engine 内部経路のみで外部 caller からは到達しない。CLI は前述の OS user 権限に依拠する。本 issue で新規の認可層を導入しない。
- **観測結果の露出範囲境界**: 観測経路（API / CLI）が外部 caller に返す情報は、engine が一次 owner として既に保持している run metadata / event log / state の投影に限定する。観測経路の都合で agent 出力 / ユーザー入力 / ファイルパス / workflow 出力等の機密データを新たに収集 / 加工 / 保存することはしない。すなわち、観測経路の出力には engine 保持データを超える範囲の追加情報を含めない。
- **CLI 入力の信頼境界**: CLI が読み取る `workflow_runs/` 配下のメタデータ / NDJSON と `workflows/` の YAML / builtin template は engine-owned かつ同一デバイス内の信頼済み入力として扱う（書き手は engine 自身 / リポジトリ管理者）。一方、CLI が caller から受け取る `run-id` 引数およびサブコマンド引数のみを外部入力として扱い、書式バリデーション / 存在確認を経た上で projection helper に渡す。
- **CLI 起動独立性境界**: CLI はデスクトップアプリ非稼働でも動作する（file-direct）。アプリ稼働中の in-memory 瞬時状態は次回同期書き込みまで CLI には可視化されないが、state 遷移と同期書き込みは [04] の atomic mutation 境界の中で揃っているため、観測可能な遅延は dispatch サイクル単位に閉じる。
- **internal command の非公開境界**: `WorkflowCommand::CompleteNode` / `FailNode` の組み立て経路は Tauri adapter / CLI / agent path に置かない（コードレビュー / モジュール配置 / pub(crate) で担保）。外部から `dispatch` に到達する経路で internal variant が現れた場合は内部不整合として `Err` に変換する（[04] adapter 境界に整合）。
- **観測値の整合性境界**: in-memory map と metadata file の同期書き込みが完了するまでの間、API（in-memory 優先）と CLI（file 経由）の観測結果が極短時間 divergence しうる。state 遷移と同期書き込みは [04] の atomic mutation 境界の中で揃え、divergence は dispatch サイクル単位に閉じる。
- **既存命名の破壊的置換境界**: `list_active_workflow_runs` / `list_completed_workflow_runs` / `list_workflow_runs_for_worktree` / `get_workflow_execution_log` / `get_workflow_execution_state` は本 issue で削除する。フロント側 invoke 呼び出しも spec 名称に揃える。後方互換 wrapper は導入しない。
- **scope の境界**: 本 issue は read-only API / CLI と `CompleteNode` / `FailNode` internal typed 化に閉じる。mutating CLI（[06]）/ Workflow Panel UI（[07]）/ structured output 提出 CLI（[08]）/ bash node ランタイム（[13]）/ main-agent narrator（[16]）は対象外。

### 実装に委ねること

- CLI binary の Cargo パッケージ配置（`src-tauri/src/bin/releash.rs` か、別 crate か、`releash_lib` を流用するか）。
- CLI が file-direct で利用する projection helper の共有形（既存 `event_projection.rs` を crate 内に公開して共用するか、CLI 用 module を切り出すか）。
- `list_workflow_runs` における active / terminal の区別表現（filter 引数か、`status` field で見せるか、別 endpoint か）。
- `get_workflow_run` の戻り値 shape（`WorkflowRunSummary` のみか、追加で `current_node` / `last_event_at` 等を含めるか）。
- run state の `WorkflowStateView` を CLI 上で整形する責務の所在（[`rust-first-logic`](../../.claude/rules/rust-first-logic.md) に従い Rust 側 presenter）。
- 既存 read-only Tauri command 5 件削除に伴うフロント側追従の具体的な置換箇所（`hooks/` の名称、`components/` の invoke 呼び出し）。
- `WorkflowCommand::CompleteNode` / `FailNode` の variant フィールド shape（`WorkflowEvent::NodeCompleted` / `NodeFailed` のフィールドと揃えるか、handler 内で event を組み立てるための最小集合に絞るか）。
- internal command 経由化に伴う既存 method（node 完了 / 失敗の現状直接呼び出し点）の置換粒度。
- CLI のサブコマンドパース実装（clap などの選定）と出力フォーマット（plain / `--json` フラグ）。
- file-direct 読み出し時の lock / 同時アクセスポリシー（read 専用なので OS の atomic write を信頼するか、advisory lock を入れるか）。
- テスト配置: read-only API は既存 Tauri command テストハーネス、CLI は tempdir 上に `workflow_runs/` 構造を作って integration テスト、internal `CompleteNode` / `FailNode` は engine `dispatch` テストハーネスで検証。

