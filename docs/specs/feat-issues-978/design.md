# `workflow` ドメイン 新アーキテクチャ移行 — あるべき姿

GitHub Issue: [#978](https://github.com/siro33950/releash/issues/978) / 親マイルストーン: [12] クリーンアーキテクチャ移行

本ドキュメントは [`docs/architecture/`](../../architecture/) の規約（README / DOMAIN / USECASE / GATEWAY / CONTROLLER / TEST）と、先行移行事例 `agent_session`（#977）を前提に、`workflow` ドメインの **移行後のあるべき姿** を定義する。実装そのものではなく、ターゲット構造・責務境界・移行順序・設計判断を確定させることを目的とする。

---

## 1. 目的とスコープ

### 目的

- `src-tauri/src/workflow/`（全 38,946 行、`engine.rs` が 17,891 行・506 関数）を、クリーンアーキテクチャの層構成（`domain` / `usecase` / `adaptor` / `infrastructure`）へ移行する。
- 旧 `workflow/` モジュールを **compatibility shim を残さず完全に削除** する（#977 と同じ no-shim 方針）。
- 「ワークフロー定義・実行・facet・承認」という責務群を、名前だけの移設ではなく **責務境界として再構成** する。

### スコープに含む（責務群）

| 責務 | 現行の主な所在 |
|---|---|
| ワークフロー定義（schema / 検証 / builtin / facet 合成） | `schema.rs`, `validation.rs`, `builtin.rs`, `builtin_facets/`, `facet.rs`, `contract.rs` |
| 実行（engine / dispatch / turn complete / parallel / run / state / event） | `engine.rs`, `run.rs`, `state.rs`, `event.rs`, `event_projection.rs`, `runtime_view.rs` |
| 承認（approval rules / pending command） | `engine.rs`（approval 系メソッド）, `pending_command*.rs`, `command*.rs` |
| 永続化・観測（storage / log / diagnostics） | `storage.rs`, `log.rs`, `diagnostics.rs` |
| CLI 観測入口（read-only） | `cli/mod.rs`（`releash workflow ...`） |
| Tauri / WebSocket 入口 | `commands.rs`（34 コマンド）, `ws_server/handlers.rs`, `protocol/workflow.rs` |

### スコープ境界（明示的に含まない / 維持する）

- **外部契約（Tauri command 名・WS message 名・主要 request/response shape）は移行に必須でない限り維持する。** 単なる整理目的の rename / shape 変更はしない（#977 の方針を踏襲）。
- CLI の read-only 観測経路は engine と IPC せず `workflow_runs/` 配下と `workflows/` を直接読む現行設計を維持する。clap AST は外部公開せずエントリは `cli::run()` に限定する。
- **永続化フォーマット（`workflow_runs/` の JSON 形状、event log 形式）は原則維持する。** ストレージ schema 変更を本移行と同時に抱えない。domain model とは gateway の mapper で変換する。
- 前提: `agent_session`（#977）完了後に着手する。`workflow` は agent_session usecase に transport 差なく接続する入口の一つである。

---

## 2. 現状の問題点

- `engine.rs` 単一ファイルに 17,891 行・506 関数が集中。`WorkflowEngine`（130メソッド規模）が定義解決・実行・状態遷移・承認・並列・永続化・broadcast を一手に担い、責務が混在している。
- domain ロジック（contract 検証、変数描画、secret マスキング、承認可否ルール、並列集約）が `engine.rs` / `validation.rs` / `contract.rs` 等に分散し、外部依存（`tauri`, ファイル I/O）と密結合している。
- `WorkflowExecution`（`engine.rs:444`）が実質の集約だが、entity として独立しておらず、in-memory state・永続化・broadcast 都合が同居している。
- Tauri コマンドが `commands.rs`（3,829 行）に集約され、engine を直接保持・直呼びしている（controller が薄くない）。
- `lib.rs` の `invoke_handler!` がコマンドを直接列挙しており、ドメイン単位の `register` 関数になっていない。

---

## 3. あるべき姿（ターゲット構造）

依存方向は規約どおり内向きのみ: `infrastructure → adaptor → usecase → domain`。

### 3.1 domain 層 — `src-tauri/src/domain/workflow/`

```text
domain/workflow/
├── mod.rs                          # 公開インターフェース
├── entities/
│   ├── mod.rs
│   └── workflow_execution/         # Aggregate（>1000行のため Aggregates パターン: DOMAIN.md）
│       ├── mod.rs                  # WorkflowExecution 構造体定義
│       ├── constructors.rs         # 生成（start 時の初期状態）
│       ├── update_status.rs        # 状態遷移（Running/Approved/Aborted/Completed 等）
│       ├── steps.rs                # step 状態の前進・履歴追加
│       ├── parallel.rs            # 並列子ノードの集約状態遷移
│       └── common.rs               # pub(super) ヘルパー
├── value_objects/
│   ├── mod.rs
│   ├── workflow_execution_state.rs # 実行状態 enum（現 state.rs WorkflowExecutionState）
│   ├── run_id.rs                   # run_id
│   ├── step_output.rs              # StepOutput / StepHistoryEntry / ChildOutputSnapshot
│   ├── contract_type.rs            # output contract の型表現
│   ├── facet_kind.rs               # FacetKind（instructions/contracts/policies/knowledge）
│   ├── reduce_strategy.rs          # 並列 reduce 戦略
│   └── outcome_commit_mode.rs      # OutcomeCommitMode
├── repository.rs                   # 永続化 trait（RunRepository / WorkflowDefinitionRepository / FacetRepository）
├── gateway.rs                      # 外部リソース trait（状態通知・event 発行・session 起動依頼）
└── services.rs                     # ドメインサービス（下記）
    （services/ サブディレクトリ化も可）
```

**ドメインサービス（#1032 で抽出）** — 複数エンティティ／値にまたがる純粋ロジックを domain に集約する:

| サービス | 現行の所在 | 責務 |
|---|---|---|
| `contract` | `contract.rs` | step 出力 contract の検証（`validate_contract_value*`）・lookup・metadata 除去 |
| `parallel` | `engine.rs` / `state.rs` の並列集約 | 並列子ノードの集約・reduce・遷移判定 |
| `secret_masker` | `engine.rs` / `log.rs` 等 | ログ・出力からの secret マスキング |
| `variable_renderer` | `engine.rs` / `facet.rs` 等 | 変数・テンプレート描画（プロンプト/コマンド補間） |
| `approval_rules` | `engine.rs` approval 系・`state.rs` `ApprovalOperations` | 承認可否・承認対象妥当性の純粋ルール |

domain は `tauri` / `git2` / `tokio` / ファイル I/O を `use` しない。検証・状態遷移・集約・承認可否は純粋関数／entity の `impl` として書く。

### 3.2 usecase 層 — `src-tauri/src/usecase/workflow/`（サブディレクトリ構成）

ドメインが大きいため USECASE.md の「サブディレクトリ化」規約に従い、単一ファイルではなく `usecase/workflow/` 配下に責務別分割する（Issue 補足の確定事項）。

```text
usecase/workflow/
├── mod.rs
├── start_usecase.rs        # ワークフロー起動（定義解決→worktree解決→初期 execution 生成→永続化→通知）
├── dispatch_usecase.rs     # コマンド dispatch（外部/内部）。pending command の振り分け
├── turn_complete_usecase.rs# agent turn 完了時の状態前進・次ステップ起動
├── parallel_usecase.rs     # 並列ステップの起動・集約・完了オーケストレーション
├── approval_usecase.rs     # 承認・却下の適用。承認対象/chat instruction の検証
├── abort_usecase.rs        # 中断（worktree 起点 / run_id 起点）
├── query_service.rs        # CQRS Query 側（run 一覧・状態・log・facet 一覧の read model 返却）
└── dto.rs                  # QueryService の Response DTO（serde, camelCase）
```

- Usecase は domain の trait（repository / gateway / services）にのみ依存。`tauri` / `git2` を直接 `use` しない。
- 複数集約・複数 repository をまたぐオーケストレーション（定義解決→worktree→execution→通知の順序制御）は usecase の責務。
- agent session 起動・送信・中断は `agent_session` usecase（#977）へ抽象 gateway 経由で依頼し、SDK/process handle 型に依存しない。
- QueryService は Usecase ではない。read model はデータソース（`workflow_runs/` 等）から `query_models` を直接組み立てて返し、`Entity → DTO` 詰め替えはしない。

### 3.3 adaptor/gateway 層 — `src-tauri/src/adaptor/gateway/workflow/`

```text
adaptor/gateway/workflow/
├── mod.rs
├── run_repository_impl.rs       # RunRepository 実装（workflow_runs/ への永続化 + event log）
├── definition_repository_impl.rs# WorkflowDefinitionRepository 実装（workflows/ 読み書き・builtin 合成）
├── facet_repository_impl.rs     # FacetRepository 実装（facet 永続化・preview）
├── query_service_impl.rs        # QueryService trait 実装（run/log/facet の read model 直接構築）
├── service_impl.rs              # gateway trait 実装（状態通知 broadcast / event 発行 / session 起動依頼）
├── command_models.rs           # 永続化用モデル + domain 型変換（現 storage.rs / run.rs / event.rs 相当）
├── query_models.rs             # Query 専用 read model（現 runtime_view.rs / protocol *View 構築の供給元）
└── service_models.rs           # 外部システム / wire 用モデル + 変換
```

- `git2` / ファイル I/O / `tauri::AppHandle` / WS broadcaster の詳細は gateway / infrastructure 内に閉じる。
- 状態通知（Tauri event / WS broadcast）は domain の通知 trait を `service_impl.rs` が実装し、infrastructure の送信実装を呼ぶ（GATEWAY.md「WebSocket の外向き送信」）。
- event log の append・projection（現 `event.rs` / `event_projection.rs` / `log.rs`）は永続化詳細として gateway/infrastructure に置く。projection から read model を組み立てる経路は `query_service_impl` / `query_models` に集約。

### 3.4 adaptor/controller 層

```text
adaptor/controller/command/workflow/
├── mod.rs              # register(builder) -> Builder（34コマンドをまとめる）
├── definition.rs       # list/get/save/delete/duplicate/open_in_editor workflow
├── run.rs              # start/abort/get_state/list_runs/get_run/get_run_log/get_run_state/resolve_*
├── facet.rs            # list/get/save/delete/duplicate/render_preview/list_summaries facet
├── step.rs             # open_workflow_step_tab / workflow_get_output
└── diagnostics.rs      # diagnose_all_cmd / get_automation_config_dir

adaptor/controller/handler/workflow/   # WS 入口（現 ws_server/handlers.rs の workflow 経路）
└── <usecase>.rs

adaptor/protocol/workflow.rs           # 現 protocol/workflow.rs を移設（*View DTO / WorkflowStateSync）
```

- controller は usecase のみを呼ぶ。QueryService / Repository を直呼びしない（読み取りも usecase 経由）。
- `command/workflow/mod.rs` に `register<R>(builder) -> Builder` を用意。`command/mod.rs` の `register_all` から呼ぶ。
- `lib.rs` の `invoke_handler!` 直接列挙を廃し、`register` 関数経由に置き換える（Tauri の `invoke_handler` が1度しか呼べない制約があれば、関数リストを集約して1回で登録する形に調整）。
- CLI 入口（`cli/mod.rs`）は read-only 観測のため、controller の薄い入口として usecase の QueryService 経路（または gateway の read model）に接続する。現行の「engine と IPC せず `workflow_runs/` を直接読む」設計は維持。

### 3.5 DI 配線 — `lib.rs` / `adaptor/controller/state.rs`

- `AppState` に `Arc<WorkflowUsecase 群>` を保持（QueryService は AppState に直接持たせず、各 usecase が内部に保持）。
- `lib.rs`（composition root）で repository_impl / query_service_impl / service_impl を生成して usecase に注入し、`builder.manage(AppState{..})`。
- 状態変更リスナー（#977 の `register_state_change_listener` パターン）で usecase 側の状態確定後に通知を発火し、controller/gateway から Tauri event / WS broadcast を送る。

---

## 4. 現行 → ターゲット マッピング

| 現行モジュール | 行数 | 移行先（あるべき層） |
|---|---:|---|
| `engine.rs`（WorkflowEngine） | 17,891 | `domain/workflow/entities/workflow_execution/`（集約）+ `domain/workflow/services`（contract/parallel/secret/variable/approval）+ `usecase/workflow/*`（start/dispatch/turn_complete/parallel/approval/abort）+ `adaptor/gateway/workflow/service_impl` |
| `WorkflowExecution`（engine.rs:444） | — | `domain/workflow/entities/workflow_execution/`（Aggregate） |
| `schema.rs`（Workflow 定義型） | 625 | `domain/workflow/value_objects` + 定義モデル（`command_models`） |
| `validation.rs` | 2,647 | domain services（純粋検証）+ usecase（手順依存検証） |
| `contract.rs` | 576 | `domain/workflow/services`（contract） |
| `facet.rs` / `builtin.rs` / `builtin_facets/` | 1,322 / 999 | domain（FacetKind/合成ルール）+ `adaptor/gateway/workflow/definition_repository_impl`（永続化・builtin 供給） |
| `state.rs`（WorkflowState 等） | 181 | VO（`workflow_execution_state`, `step_output`）+ Query 側 read model |
| `run.rs` / `storage.rs` | 2,315 / 859 | `adaptor/gateway/workflow/run_repository_impl` + `command_models` |
| `event.rs` / `event_projection.rs` / `log.rs` | 679 / 2,129 / 1,275 | `adaptor/gateway/workflow`（永続化・projection）+ `query_models` |
| `runtime_view.rs` | 194 | `usecase/workflow/dto.rs` + `query_models` |
| `pending_command*.rs` / `command*.rs` | 計 ~3,000 | usecase（dispatch/approval）+ gateway（永続化） |
| `diagnostics.rs` | 1,679 | usecase（診断手順）+ gateway/infrastructure |
| `resolver*.rs` / `route_context.rs` / `worktree.rs` | 計 ~430 | domain gateway trait + gateway 実装（定義/worktree 解決） |
| `commands.rs`（34 Tauri commands） | 3,829 | `adaptor/controller/command/workflow/*` |
| `protocol/workflow.rs`（*View） | — | `adaptor/protocol/workflow.rs` |
| `ws_server/handlers.rs`（workflow 経路） | — | `adaptor/controller/handler/workflow/` |
| `cli/mod.rs`（workflow 観測） | — | controller 入口として usecase/QueryService に接続（read-only 設計維持） |

> 行数・配置は現状調査に基づく見積もりであり、実装時に責務凝集度で再判断する（DOMAIN.md「行数だけでなく責務の凝集度で判断」）。

---

## 5. サブIssue別 移行計画（依存順）

各サブIssueはそれぞれビルドとテストが通る単位とする。

| Issue | 内容 | 成果物 | テスト必須層 |
|---|---|---|---|
| **#1031** | 値オブジェクト・エラー型・補助 trait を抽出 | `domain/workflow/value_objects/*`, `WorkflowError`(thiserror), repository/gateway trait のシグネチャ | domain |
| **#1032** | ドメインサービス抽出 | `domain/workflow/services`（contract / parallel / secret_masker / variable_renderer / approval_rules） | domain |
| **#1033** | `WorkflowExecution` を Aggregate 化 | `domain/workflow/entities/workflow_execution/`（constructors/update_status/steps/parallel/common） | domain |
| **#1034** | usecase（Command）責務別分解 | `usecase/workflow/{start,dispatch,turn_complete,parallel,approval,abort}_usecase.rs` | usecase |
| **#1035** | query_service（CQRS Query 側）抽出 | `usecase/workflow/query_service.rs` + `dto.rs`, `adaptor/gateway/workflow/query_service_impl.rs` + `query_models.rs` | usecase / gateway |
| **#1036** | gateway / infrastructure 実装 | `adaptor/gateway/workflow/*`（repository_impl / service_impl / command_models / service_models）、永続化・event 発行 | gateway |
| **#1037** | controller 移行 + 旧モジュール削除 | `adaptor/controller/command/workflow/`（register）, `handler/workflow/`, protocol 移設, `lib.rs` register 化, **`src-tauri/src/workflow/` 完全削除** | controller は柔軟 |

### 移行中のコンパイル維持戦略

no-shim 方針のため、移行途中で旧 `workflow/` と新 `domain|usecase|adaptor/workflow` が一時的に併存する。#1031〜#1036 では新コードを追加しつつ旧コードを残し、各段階でビルド・テストを通す。**旧モジュールの削除と外部呼び出し側（`workflow_state_events`, `session_commands`, `workflow_step_lifecycle*`, `workflow_state_presenter`, `ws_server/handlers`, `cli/mod`, `lib.rs`）の切り替えは #1037 で一括** して行う。これにより部分移行中のコンパイル不能期間を最小化する。

---

## 6. 主要な設計判断

### 6.1 Context boundary

`workflow` を単一 bounded context として扱う。ワークフロー定義・実行・facet・承認の意味論（状態遷移・contract・並列集約・承認可否）を domain/usecase に集約する。Claude/Codex SDK 実行、PTY、ファイル I/O、Tauri event、WS broadcast は domain に入れず gateway/infrastructure に置く。agent セッションの起動・送信・中断は `agent_session` usecase へ抽象 gateway 経由で依頼する。

### 6.2 Entity か read model か（DOMAIN.md「誰の都合か」）

- `WorkflowExecution` とその状態遷移・step 履歴・並列集約は domain Entity / VO（実行の意味論はアプリの都合で決まる）。
- `protocol/workflow.rs` の `*View`（`WorkflowStateView`, `WorkflowDefinitionView` 等）と `runtime_view.rs` は **表示・転送の都合で形が決まる read model / DTO**。domain には置かず、`usecase/workflow/dto.rs`（Response）と `adaptor/gateway/workflow/query_models.rs`（読み取り供給元）に配置する。`query_service_impl` がデータソースから直接構築し、`Entity → DTO` 詰め替えはしない。

### 6.3 永続化互換性（DB / storage）

新しい storage backend は導入しない。`workflow_runs/` の JSON 形状・event log 形式は維持し、domain model とは `command_models` の mapper で変換する。移行を storage schema 変更と切り離す。

### 6.4 外部契約の維持（Interface）

Tauri command 名・WS message 名・主要 request/response shape は移行に必須でない限り維持する。内部で新 usecase / VO / typed error を使っても controller/handler が既存契約へ変換する。整理目的だけの protocol rename / shape 変更はしない。

### 6.5 状態と通知の一貫性（Cross-cutting）

usecase が永続状態と runtime 状態の更新を確定した後に公開状態を導出し、controller/gateway から Tauri event / WS broadcast を送る。repository / service が個別に通知を発火する設計にはしない（desktop / remote / CLI 観測の順序・意味論を揃える）。

### 6.6 承認・中断のオーケストレーション

承認可否・承認対象妥当性の純粋ルールは domain（`approval_rules`）。承認適用・chat instruction 検証・中断（worktree 起点 / run_id 起点）の手順制御は usecase。runtime への cancel/abort 操作は gateway に閉じる。

---

## 7. リスク

- **大規模 no-shim 移行**: `engine.rs` 17,891 行・呼び出し側多数（`workflow_state_events` / `session_commands` / `workflow_step_lifecycle*` / `ws_server` / `cli` / `lib.rs`）を #1037 で同時切り替えするため差分が大きい。サブIssue分割で各段階のビルド・テスト通過を担保する。
- **永続化 mapper 互換性**: `workflow_runs/` JSON と event log 形状を維持しつつ domain model を分離するため、`command_models` mapper の欠落は run 復元・履歴・CLI 観測の回帰につながる。
- **event projection の二重実装回避**: engine と CLI 双方が event から状態を再構成する（`reconstruct_state_from_events`）。README.md「同じ操作の実装は1つに集約」に従い、projection と read model 構築を gateway/query 側の単一経路に集約する。
- **既存 edge case の扱い**: desktop / remote / CLI で behavior の範囲に意味論を揃える方針のため、旧実装固有の edge case をすべて保存するわけではない。既存テストが旧 edge case を固定している場合、仕様として残すか旧実装由来として整理するか判断する。
- **WorkflowEngine 状態の分解**: in-memory `executions` / `session_workflow_refs` / `run_store` は永続状態と transient runtime state が同居している。domain Entity（永続意味論）と gateway の transient state（in-memory index）への分離境界を #1033 / #1036 で誤ると、active run 解決（`find_by_worktree` 系）の挙動回帰につながる。
