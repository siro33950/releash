## 要求

**種別**: リファクタリング（互換性境界の確立 / 旧 schema・旧 state/log shape の廃止）

**ゴール**:
Workflow Engine Evolution の milestone [02]「Normalized Workflow」を完了する。本マイルストーンでは、進化計画全体の前提だった「旧 YAML / 旧 `WorkflowState` JSON / 旧 NDJSON event log の互換維持」を**破棄**し、旧概念を残さない構造に置換する。

具体的には次の 2 つを同時に達成する:

1. **互換性境界の構造的確立**: future core モジュール（`engine.rs` / `contract.rs`）と `schema.rs` の新 schema 型が、旧 schema 型（`Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）を一切参照しない状態を作る
2. **旧 shape の廃棄**: 旧 schema 型・旧 `WorkflowState` JSON shape・旧 NDJSON event log shape を**削除**し、新 schema / 新型に置換する。built-in workflow YAML（`spec-driven-development.yml`）も新 schema で書き直す

互換破棄方針により旧→新の変換工程が消えるため、plan doc [02] が当初予定していた `workflow/normalized.rs`（旧 schema を新 NodeDefinition に変換する層）は**新設しない**。`schema.rs` が新 `Workflow`（template 定義）と `NodeDefinition` を直接 YAML deserialize 先として保持し、engine もそれを直接消費する。新型を `NormalizedWorkflow` などの過渡的な命名で呼ばず、旧 `Workflow` 削除後は単に `Workflow` を再利用する（boundary doc の `WorkflowRun` / `NodeExecution` 等の実行インスタンス系モデルとは語彙が明確に分離される）。

後続マイルストーン [03]〜[16] はいずれも新規モジュールの**追加**が主で、旧 shape の廃棄を担わないため、本マイルストーンで完全な置換まで行う。

**背景**:
milestone [01]（GitHub Issue #1003）で north star ドキュメント `docs/workflow-engine-model-boundary.md` を固定し、未来形 5 モデル（`WorkflowRun` / `NodeDefinition` / `NodeExecution` / `WorkflowCommand` / `WorkflowEvent`）の責務とフィールド、既存モジュールの future core / compatibility adapter 分類を確定した。

当初 plan doc / boundary doc は「旧 YAML / 旧 JSON / 旧 NDJSON 互換維持」を進化計画全体の不変条件として記述していた。しかし Releash は v0.3.x 段階で、`workflow_definition` を埋め込んだ進行中の state / 過去 NDJSON ログは破棄して問題ない運用前提であり、built-in workflow YAML も本リポジトリ内資産のため書き換え可能である。互換維持の制約を恒久的に背負うコストが正当化されないため、本マイルストーンで境界面より上の compat 責務を縮退させ、新 schema / 新型のみが流通する構造に統一する。

現状、`engine.rs` は `crate::workflow::schema::{Workflow, Step, ...}` を直接 import し、内部メソッドの引数も `&Step` を受け取る（line 18, 2945, 3022, 3291 ほか）。`state.rs::WorkflowState.workflow_definition: Workflow`（line 22）と `log.rs::WorkflowLogEvent::WorkflowStarted.workflow_definition: Option<Workflow>`（line 25）も旧 `Workflow` 型を引きずっている。これらを 1 つのマイルストーンで一掃する。

**スコープ（GitHub Issue #1005 / Milestone 58「[02] Workflow Engine Evolution - Normalized Workflow」対応範囲）**:

### A. 新 schema / 新型の確立

- `src-tauri/src/workflow/schema.rs` を**新 schema 専用**に書き換える
  - 旧型 `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` を**削除**する
  - 新型 `Workflow`（template 定義）/ `NodeDefinition` および関連する node 種別ごとの設定型を Rust の型として導入する（boundary doc 93-114 行の暫定フィールドを基準）
    - `Workflow` 型（name / description / builtin / nodes: Vec<NodeDefinition>）— 旧 `Workflow` を削除した上で同名を再利用する。旧 `steps` フィールドは `nodes` に置き換える
    - `NodeDefinition` フィールド: `node_name` / `node_type` / `agent_config` / `command_config` / `approval_config` / `parallel_children` / `transition_rules` / `cycle_guard` / `resets_cycle_for` / `model` / `permission`
    - `node_type` 列挙: `agent` / `bash` / `approval` / `parallel`（aggregate は parallel の振る舞いに集約）
  - 新 YAML schema を `node_type` ベースで定義する（`mode` フィールドは廃止し、`type: agent | bash | approval | parallel` に統一）。YAML は `Workflow` / `NodeDefinition` に直接 deserialize される
  - facet 参照（policy / knowledge / instruction / output_contract）は YAML deserialize の前後で `facet.rs` を呼んで解決し、`NodeDefinition.agent_config` に解決済み値を保持する。schema 層に未解決の facet ref を残さない
  - `workflow/normalized.rs` は**新設しない**（旧→新の変換層が不要なため）
- 既存型を import している箇所はすべて新型に書き換える（schema.rs から旧型は消えるため、コンパイル必須）

### B. built-in YAML の書き換え

- `src-tauri/src/workflow/builtin/spec-driven-development.yml` を新 schema に書き換える
  - `mode: approval` → `type: approval`
  - `mode: auto` → `type: agent`
  - `parallel: [...]` block と `aggregate: ...` は新 schema の `type: parallel` + 子 node 群 + aggregate 設定に書き換える
  - 既存挙動を完全に再現する（step 数・遷移先・cycle guard・aggregate 条件すべて等価）

### C. future core 側

- `src-tauri/src/workflow/engine.rs` から `schema::*` の import を**完全に除去**する
  - 関数シグネチャ・内部メソッド・テストコード（unit test 含む）が `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` を直接参照しない状態にする
  - 中核状態遷移（step 解決・分岐・parallel/aggregate・cycle guard・contract 検証など）は新 `Workflow` / `NodeDefinition` のみを参照する
  - 既存ヘルパー（line 2945/3022/3291 等の `step: &Step` を受け取る関数）は `node: &NodeDefinition` 受け取りに置き換える
  - engine 内のテストフィクスチャ（line 4667/5141）は `NodeDefinition` を直接組み立てる形に書き換える
- `src-tauri/src/workflow/contract.rs` は現状 `schema::*` を import していない（確認済み）。`NodeDefinition.agent_config` の output_contract 等から呼ばれる形を維持

### D. state.rs / log.rs の shape を新型に置換

- `src-tauri/src/workflow/state.rs::WorkflowState.workflow_definition: Workflow` の型自体は同名で残るが、参照先が**新 `Workflow`**（NodeDefinition ベース）に置き換わる
  - 既存 `WorkflowState` JSON 互換は**維持しない**（在庫 state は破棄前提）
  - フィールド内部の `step_*` 命名は本マイルストーンでは維持（vocabulary を `node_*` / `WorkflowEvent` に寄せるのは [04] の責務）。型の置換のみ行う
- `src-tauri/src/workflow/log.rs::WorkflowLogEvent::WorkflowStarted.workflow_definition: Option<Workflow>` を新型に置換する
  - NDJSON 互換は**維持しない**（在庫ログは破棄前提）
  - 同様に `step_*` 命名は本マイルストーンで残す（[04] で WorkflowEvent vocabulary に寄せる）

### E. engine の caller path

- `src-tauri/src/workflow/commands.rs` / `src-tauri/src/agent_commands.rs` / `src-tauri/src/session_commands.rs` / `src-tauri/src/session/mod.rs` の engine 駆動箇所を新 schema 経由に書き換える
- `src-tauri/src/workflow_state_presenter.rs` も `WorkflowState` の新型に対応する

### F. compat adapter 群の追従

- `validation.rs` — 新 schema を検証する形に書き換える（旧 schema の検証は削除）。YAML ロード時に呼ばれる
- `facet.rs` — facet 参照解決を引き続き担う。新 schema での facet ref を解決して `NodeDefinition.agent_config` に流す
- `storage.rs` — YAML I/O。新 schema を load / save する形に書き換える
- `diagnostics.rs` — 新 schema を対象とする診断に書き換える
- `runtime_view.rs` — 新型 `WorkflowState` に対応
- `builtin.rs` / `builtin_facets/` — built-in YAML 書き換えに伴う追従

### G. 進化計画ドキュメントの更新

- `docs/workflow-engine-evolution-plan.md` の「互換性境界」節および各マイルストーンの完了条件記述を更新する
  - 「既存 YAML は有効なままにする」「既存 `WorkflowState` JSON は deserialize できるようにする」「既存 Tauri command は動かし続ける」を**[02] で旧 schema・旧 shape を廃止する**方針に書き換える
  - `Step` から `NodeDefinition` への変換例（plan doc 100-106 行）は、変換ではなく**新 schema の構文として直接 `node_type` を表現する**説明に書き換える
  - [02] の作業項目・完了条件を、旧 schema 廃止・built-in YAML 書き換え・state/log shape 置換を含む内容に更新する
- `docs/workflow-engine-model-boundary.md` の「前提となる互換性境界」節（22-26 行）を更新する
  - 旧 YAML / 旧 JSON / 旧 Tauri command の互換維持を前提とする記述を、[02] 完了後の状態（旧 schema 削除済み）に整合させる
  - 「Old Tauri command / old UI / old YAML → compat adapter → Run/Node/Command/Event」の構造図は、旧 schema 廃止後でも user-authored YAML が新 schema として入力される経路と整合的なため、図そのものは維持する（compat adapter の責務範囲のみ更新）

### H. テスト

- `schema.rs` 内に unit test を配置する:
  - `type: agent` / `type: approval` / `type: bash`（型のみ、実行系は [13]）/ `type: parallel` の variant 別 load
  - `parallel` 子 node と aggregate 設定の load
  - transition_rules / cycle_guard / resets_cycle_for / facet 参照解決 / model / permission など override の保持
- 互換性スナップショット相当のテスト:
  - `spec-driven-development.yml` を新 schema として load した結果が、書き換え前と等価な実行構造を持つ（step 数・各 node の `node_type`・並列構成・aggregate 設定・遷移先・cycle guard）
  - 書き換え前後の実行挙動が等価（既存の engine 実行テストが新 schema 上で通る）
- 旧 schema を扱っていた既存テスト群（`validation.rs` / `diagnostics.rs` / `facet.rs` / `engine.rs` / `commands.rs` / `storage.rs` の test mod）は、新 schema に書き換える
- 既存 `WorkflowState` JSON / NDJSON deserialize 互換テスト（存在すれば）は**削除**する

**スコープ外（後続マイルストーンに委ねる / 本マイルストーンでは触らない）**:

- `WorkflowState` 型自体の `WorkflowRun` へのリネームと `run_id` 主語管理（[03] Run Store / Run ID）
- `WorkflowLogEvent` の vocabulary 寄せ（`step_*` → `node_*` / `WorkflowEvent` 命名）（[04] Command / Event Boundary）
- `WorkflowCommand` typed 入口の導入（[04]）
- read-only Run API / CLI（[05]）、mutating CLI（[06]）、Workflow Panel（[07]）、Output CLI（[08]）
- `type: bash` の**実行系統**の新規追加（[13]）。型・load・schema 上の対応は本マイルストーンに含めて差し支えないが、bash 実行ロジックは追加しない
- 新規 template の追加（[14]）、Skill（[15]）、Main Agent Mediation（[16]）
- 旧 Tauri command 入口の削減・廃止（command の入口・出口形は本マイルストーンでは維持）
- フロントエンド（React/TypeScript）の workflow 表示 UI の本格的な再設計（本マイルストーンは Rust 側構造の置換に限定）。ただし、`workflow_state_presenter.rs` 経由で TypeScript 側に流れる DTO 形が新型に追従するため、フロントエンド側の型定義（TypeScript）と表示用フォーマットは新型に合わせて最小限の追従修正を行う
- user-authored YAML（リポジトリ外で利用者が書いた workflow ファイル）に対するマイグレーションツール — 必要に応じて利用者が手で新 schema に書き換える前提とし、自動マイグレーション機構は本マイルストーンでは作らない

**制約**:

- runtime 振る舞いは `spec-driven-development.yml` の実行において**書き換え前と等価**であること（step 数・遷移・並列・aggregate・cycle guard・facet 解決結果がすべて一致）
- 既存 `WorkflowState` JSON / 既存 NDJSON event log の在庫はリリース時に破棄される前提を許容する（互換は維持しない）
- 解決済み facet 値（policy / knowledge / instruction / output_contract）および workflow 定義本体の保存・露出方針は本マイルストーンで拡張しない
  - state / log への保持範囲は既存と同じ（`workflow_definition` フィールドの型置換のみで、保存方針自体は変更しない）
  - frontend DTO への露出範囲は既存を超えない
  - ログの外部送信や追加露出は本マイルストーンでは行わない
- workflow の load / 開始 / 進行 / approval に関する認可境界は本マイルストーンでは変更しない（既存 Tauri command / session の認可前提をそのまま維持し、追加の認可機構は導入しない）
- 旧 schema 型（`Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）は本 PR 終了時点で codebase に存在しない
- future core モジュール（`engine.rs` / `contract.rs`）は旧 schema 型を一切 import しない（`grep` で 0 件であることを確認できる）。`workflow/normalized.rs` は新設しない
- ドキュメントは日本語で記述する（既存 plan doc / boundary doc と同じ）
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を満たす
- フロントエンド側に変更が及ぶ場合は `pnpm lint` / `pnpm test` / `pnpm build` も成功する

**完了条件**:

- `src-tauri/src/workflow/schema.rs` から旧型（旧 `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）が**削除**され、新 `Workflow`（NodeDefinition ベース）/ `NodeDefinition` および関連型に置き換わっている（`Workflow` という名前自体は再利用するが、フィールド構成は新しい）
- `src-tauri/src/workflow/normalized.rs` は新設されていない（旧→新の変換層を持たない構造になっている）
- `src-tauri/src/workflow/engine.rs` の `schema::*` import が、旧型を一切参照しない状態になっている（`Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` を `grep` して 0 件）
- engine の全 caller path（`workflow/commands.rs` / `agent_commands.rs` / `session_commands.rs` / `session/mod.rs` 等）が新 schema 経由で engine を駆動する形に揃っている
- `state.rs::WorkflowState.workflow_definition` と `log.rs::WorkflowLogEvent::WorkflowStarted.workflow_definition` が新型を保持している
- `src-tauri/src/workflow/builtin/spec-driven-development.yml` が新 schema で書き直され、既存挙動と等価に実行できる
- node_type（agent / approval / parallel / bash）別の load unit test が `schema.rs` 内に存在する
- `docs/workflow-engine-evolution-plan.md` と `docs/workflow-engine-model-boundary.md` の互換維持記述および `normalized.rs` 新設前提が、新方針（旧 schema 廃止 / `normalized.rs` 不要）に整合して更新されている
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（src-tauri 配下）および `pnpm lint` / `pnpm test` / `pnpm build`（プロジェクトルート）が成功する

## 振る舞い定義

```gherkin
Feature: 正規化されたワークフロー定義

  Rule: ワークフローはnode種別ごとの振る舞いを持つ単位の連なりとして表現される

    Scenario: ワークフロー作成者がエージェント種別のnodeを記述する
      Given ワークフロー作成者がワークフロー定義を編集している
      When エージェント種別のnodeを定義する
      Then そのnodeはエージェント駆動の作業単位として扱われる

    Scenario: ワークフロー作成者が承認種別のnodeを記述する
      Given ワークフロー作成者がワークフロー定義を編集している
      When 承認種別のnodeを定義する
      Then そのnodeは利用者の承認を必要とする待機単位として扱われる

    Scenario: ワークフロー作成者がbash種別のnodeを記述する
      Given ワークフロー作成者がワークフロー定義を編集している
      When bash種別のnodeを定義する
      Then そのnodeはシェル実行に相当する作業単位として保持される

    Scenario: ワークフロー作成者が並列種別のnodeを記述する
      Given ワークフロー作成者がワークフロー定義を編集している
      When 並列種別のnodeとその子node群および集約条件を定義する
      Then そのnodeは子node群を並列に走らせ集約条件に従って収束する単位として扱われる

  Rule: nodeは遷移・サイクル制御・facet・実行設定をnode自身に紐づけて保持する

    Scenario: 作成者がnodeに遷移先・サイクル境界・モデルや権限の上書きを記述する
      Given ワークフロー作成者がnodeを編集している
      When 遷移条件・サイクル境界・サイクルリセット対象・モデル・権限を指定する
      Then それらはそのnode固有の振る舞いとして保持され、実行時に反映される

    Scenario: 作成者がnodeのエージェント設定で共通facetを参照する
      Given ワークフロー作成者がエージェント種別のnodeを編集している
      When ポリシー・知識・指示・出力契約のfacet参照を記述する
      Then それらは解決済みの設定としてnodeに取り込まれた状態で扱われる

  Rule: 組み込みワークフローの実行挙動は新しい定義表現に置き換わっても利用者から見て等価である

    Scenario: 利用者が組み込みのspec-driven-developmentワークフローを開始する
      Given 組み込みワークフロー spec-driven-development が提供されている
      When 利用者がこのワークフローを開始して進行させる
      Then 利用者は従来と同じ作業順序・分岐・並列構成・集約結果・サイクル制御を体験する

  Rule: 旧表現の進行中状態と過去ログは新バージョン移行時に引き継がれない

    Scenario: 旧表現の進行中ワークフローを抱えた利用者が新バージョンへ移行する
      Given 旧表現で記録された進行中のワークフロー状態が利用者の環境に残っている
      When 利用者が新バージョンに移行する
      Then 旧表現の進行中状態は引き継がれず、新たな進行から開始するものとして扱われる

    Scenario: 旧表現のイベントログを抱えた利用者が新バージョンへ移行する
      Given 旧表現で記録されたワークフローイベントログが利用者の環境に残っている
      When 利用者が新バージョンに移行する
      Then 旧表現のイベントログは新バージョンの参照対象とはならない

  Rule: 新schemaに適合しないワークフロー定義は実行対象として受理されない

    Scenario: ワークフロー作成者が新schemaに適合しない定義を読み込ませる
      Given ワークフロー作成者が新schemaに適合しないワークフロー定義を保持している
      When 新バージョンでその定義を読み込もうとする
      Then その定義は実行可能な定義として受理されず、作成者に問題が示される

  Rule: 利用者が独自に作成したワークフロー定義は利用者自身が新しい表現へ書き換える

    Scenario: 利用者が旧表現で書いた独自ワークフロー定義を新バージョンに持ち込む
      Given 利用者が旧表現で書いた独自のワークフロー定義を保持している
      When 新バージョンでその定義を読み込もうとする
      Then その定義は新schemaとして受理されず、利用者は新しい表現で書き直さない限り実行に進めない
```

## アーキテクチャ概要

### 責務配置

- **`workflow/schema.rs`（新 schema 専用）**: 新 `Workflow`（template 定義）/ `NodeDefinition` / node 種別ごとの設定型を保持し、YAML の直接 deserialize 先となる。担当しないこと: 旧型の保持、旧→新の変換、facet 参照の解決ロジック本体、実行時状態、ログ shape
- **`workflow/facet.rs`**: facet 参照（policy / knowledge / instruction / output_contract）の解決を担う。schema load 経路から呼ばれ、`NodeDefinition.agent_config` に解決済み値を流す。担当しないこと: schema 型の定義、未解決 ref を schema に残すこと
- **`workflow/validation.rs`**: 新 schema 単位での静的妥当性検証（node 種別ごとの必須フィールド・遷移先の到達可能性・cycle_guard 整合等）。担当しないこと: 旧 schema 検証、実行時整合性
- **`workflow/diagnostics.rs`**: 新 schema を対象とした診断（編集者向けの問題提示）。担当しないこと: 旧 schema 診断
- **`workflow/storage.rs`**: YAML I/O。新 schema の load / save を担う。担当しないこと: shape 変換、互換読み込み
- **`workflow/builtin.rs` / `builtin/spec-driven-development.yml`**: 組み込みワークフローの提供。YAML は新 schema で記述される。担当しないこと: 旧 schema フォーマットの保持
- **`workflow/engine.rs`（future core）**: 新 `Workflow` / `NodeDefinition` のみを参照して状態遷移（node 解決・分岐・parallel/aggregate・cycle guard・contract 検証）を駆動する。担当しないこと: 旧 schema 型の import、shape 変換
- **`workflow/contract.rs`（future core）**: output_contract 検証ロジック。`NodeDefinition.agent_config` の output_contract から呼ばれる。担当しないこと: schema 型の保持
- **`workflow/state.rs`**: 実行時の `WorkflowState` を保持する。`workflow_definition` は新 `Workflow` 型を指す。担当しないこと: 旧 JSON shape の互換維持、vocabulary の `node_*` 改名（[04] 範囲）
- **`workflow/log.rs`**: `WorkflowLogEvent` を定義し、`WorkflowStarted.workflow_definition` は新 `Workflow` 型を保持する。担当しないこと: 旧 NDJSON shape の互換、event vocabulary の改名（[04] 範囲）
- **`workflow/runtime_view.rs`**: 実行時状態の表示用ビュー。新型 `WorkflowState` に追従する。担当しないこと: schema 定義
- **engine caller（`workflow/commands.rs` / `agent_commands.rs` / `session_commands.rs` / `session/mod.rs`）**: 新 schema 経由で engine を駆動する。担当しないこと: 旧型の中継、shape 変換
- **`workflow_state_presenter.rs`**: `WorkflowState` を frontend 向け DTO に整形する。新型に追従する。担当しないこと: 表示判定ロジックの肥大化、frontend 側の判断混入
- **frontend（TypeScript）**: DTO の表示・入力受付。新 DTO 型に最小限追従する。担当しないこと: schema 変換、ロジック保持（rust-first-logic 原則）

### データ/通信フロー

- **YAML load**: file / built-in → `storage.rs` → `schema.rs`（deserialize 先: 新 `Workflow` / `NodeDefinition`）→ `facet.rs`（facet ref 解決して `agent_config` に格納）→ `validation.rs`（静的検証）→ engine 駆動可能な `Workflow` インスタンス
- **workflow 開始/進行**: frontend → Tauri command（`commands.rs` / `session_commands.rs` 等）→ `engine.rs`（新 `Workflow` / `NodeDefinition` を参照して `WorkflowState` を更新）→ `log.rs`（新 shape で記録）→ `workflow_state_presenter.rs` → frontend
- **organic な node 解決**: engine 内で `WorkflowState.current_*` から `Workflow.nodes` を引いて `NodeDefinition` を取得 → `node_type` 分岐で振る舞いを決定（agent / approval / bash / parallel）
- **built-in 読み込み**: 起動時 `builtin.rs` → 同梱 YAML を `schema.rs` 経由で deserialize → 他の user-authored YAML と同じ load 経路に合流
- **旧在庫の扱い**: 旧 JSON `WorkflowState` / 旧 NDJSON ログは load 経路に存在しない（互換 deserializer を持たない）。利用者環境では新バージョン起動時に破棄される前提

### 状態Owner

- **新 schema 型定義（`Workflow` / `NodeDefinition` 等）**: `workflow/schema.rs`
- **facet 解決結果（`NodeDefinition.agent_config` 内の解決済み policy / knowledge / instruction / output_contract）**: load 時に `facet.rs` が解決し、以降は `NodeDefinition` が保持する（schema 層に未解決 ref を残さない）
- **進行中の workflow 実行状態（`WorkflowState`）**: `workflow/state.rs`（型）／実行時の所有は engine / session 層
- **イベントログ shape（`WorkflowLogEvent`）**: `workflow/log.rs`
- **組み込み workflow YAML 資産**: `workflow/builtin/` 配下（リポジトリ内）
- **engine の状態遷移判断**: `workflow/engine.rs`（新 `NodeDefinition` のみを根拠とする）
- **frontend 表示用 DTO 形**: `workflow_state_presenter.rs`（Rust 側で確定し、frontend は受け取って表示するのみ）

### 境界

- **future core / schema 境界**: `engine.rs` / `contract.rs` は旧型（旧 `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）を一切 import しない。新 schema のみ参照
- **schema / facet 境界**: 未解決 facet ref は schema 層に残さない。load パイプライン内で `facet.rs` が解決を完了する
- **schema / 変換層境界**: 旧→新の変換層（`workflow/normalized.rs`）は新設しない。schema が直接 deserialize 先となり、変換工程を持たない
- **schema / 実行インスタンス境界**: `Workflow` / `NodeDefinition` は template 定義。`WorkflowRun` / `NodeExecution` 等の実行インスタンス系語彙とは分離される（実行インスタンス系の正式リネームは [03][04] の範囲）
- **互換境界の縮退**: 旧 YAML / 旧 JSON state / 旧 NDJSON log の互換は本マイルストーンで放棄。compat adapter 層は user-authored YAML（新 schema）の入力経路と built-in YAML 提供経路のみに縮退
- **入力信頼境界**: built-in YAML（`workflow/builtin/` 配下）はリポジトリ内資産として信頼される入力。user-authored YAML / 外部ファイル YAML は外部入力として扱い、`validation.rs` による静的検証を必須経路として通過させる
- **frontend / backend 境界（rust-first-logic）**: schema 解釈・遷移判断・facet 解決・検証はすべて Rust 側。frontend は DTO の表示と入力受付のみ
- **vocabulary 境界（本マイルストーン）**: `state.rs` / `log.rs` 内の `step_*` 命名は本マイルストーンでは維持。`node_*` / `WorkflowEvent` 語彙への寄せは [04] の責務として残す

### 実装に委ねること

- 新 `Workflow` / `NodeDefinition` の Rust struct フィールド名・型・derive 構成の細部（boundary doc 93-114 行の暫定フィールドに整合していれば自由）
- `node_type` を Rust 側で表現する enum 名・variant 名・serde tag 名（YAML 上は `type: agent | bash | approval | parallel` で統一すること以外は自由）
- `agent_config` / `command_config` / `approval_config` 等の node 種別別設定型の構造体名と内部フィールド
- facet 参照解決ロジックの呼び出しタイミング（schema deserialize の post-process として行うか、storage load の一工程として行うか等）の具体手順
- engine 内ヘルパー関数の rename 戦略（`step: &Step` 受け取りから `node: &NodeDefinition` 受け取りへ置換する際の関数名・分割粒度）
- engine 内テストフィクスチャの `NodeDefinition` 組み立てヘルパーの抽出範囲
- `validation.rs` / `diagnostics.rs` の検証ルール表現方法（個別関数 / ルールテーブル等）
- `workflow_state_presenter.rs` の DTO 構造体の内部分割
- frontend 側 TypeScript 型の生成/手書き運用、表示用フォーマット関数の配置
- 単体テストの具体的なケース分割・テスト名・フィクスチャ配置（schema.rs 内 / 各モジュール内のいずれか、命名規約に従う範囲）
- built-in YAML 書き換え時の YAML フォーマット詳細（インデント・コメント配置・キー順）。等価な実行構造を保つ範囲で自由
- 進化計画ドキュメント・boundary ドキュメントの段落構成や説明の具体的な書き方（記述すべき方針変更点が反映されていれば自由）

