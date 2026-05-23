# ワークフローエンジン発展計画

この文書は、Releash のワークフローエンジンを実装前にどう発展させるかを整理した設計メモである。

目的は、Releash のエンジンを Archon などの外部 OSS runner に置き換えることではない。Rust 側が所有する現在のワークフローランタイムを維持しつつ、Releash に合う運用パターンだけを取り込む。

取り込む対象は、テンプレート、Agent 向け Skill、明示的な Node type、決定論的な CLI/API command、Run 管理、構造化出力の提出、validation gate、Workflow 専用 UI パネルである。

## 背景

現在のワークフローエンジンには、すでに価値のある要素がある。

- YAML による workflow 定義
- `approval` / `auto` / `interactive` step mode
- 並列 step と aggregate 判定
- cycle guard
- output contract と repair prompt
- desktop UI / remote UI への workflow state 同期
- NDJSON の workflow event log

課題は、workflow の実行が主に chat session と worktree-scoped state として表現されていることにある。step session は本来 1 つの workflow run に属する実行単位だが、通常の自由対話 session と同列の tab として見えやすい。

その結果、UI 上の主語と engine 上の主語がずれる。将来 CLI、Skill、structured output、承認操作を足すほど、このずれは大きくなる。

次の設計では、workflow run を第一級の操作対象にする。

## プロダクト方針

Releash は、すでに選択済みの worktree の中で workflow を決定論的な実行レールとして扱う。

```text
User / Main Agent / UI / Remote
        |
        v
Releash CLI/API command boundary
        |
        v
Workflow Run Manager
        |
        v
Workflow Engine
        |
        v
Node Executions
  - agent
  - bash
  - approval
  - parallel
  - aggregate
```

workflow engine は状態遷移の唯一の権威であり続ける。Agent や UI は action を要求できるが、workflow state を直接決めない。

## 採用するもの

| 項目 | 判断 | 方針 |
| --- | --- | --- |
| テンプレート | 採用 | Releash native の built-in workflow template を追加する。PR/Issue 系は直接 GitHub lifecycle に接続せず、汎用テンプレートとして扱う。 |
| Skill | 採用 | Agent に「いつ Releash workflow を使うか」「どの CLI command を叩くか」を教える instruction/skill file を提供する。 |
| Node 概念 | 採用 | step に明示的な実行種別を持たせる。初期対象は `agent`, `bash`, `approval`, `parallel`, `aggregate`。必要になれば `loop` や `output/form` を追加する。 |
| Bash / validation gate | 採用 | test、lint、validation を決定論的な command node として実行する。exit code や structured result で分岐できるようにする。 |
| CLI | 採用 | UI、Agent、Remote が共有する command boundary にする。 |
| Run / Execution 管理 | 採用 | workflow 実行を `run_id` で扱う。status、logs、approval、abort などの主語を run にする。 |
| OutputForm / structured output CLI | 採用 | step agent が文章抽出だけに頼らず、CLI/API 経由で typed output を提出できるようにする。 |
| Command Center / Dashboard | 採用 | 右パネルを Review / Workflow で切り替え可能にし、Workflow 側に run history、timeline、step detail、step conversation を置く。 |
| Main Agent 仲介 | 採用 | main agent が進捗報告や承認依頼を行う。ただし user decision は typed command として engine に戻す。 |

## 採用しないもの

| 項目 | 判断 | 理由 |
| --- | --- | --- |
| Worktree isolation | 今回は採用しない | Releash は worktree を作成・選択してから task を渡す設計である。workflow 起動時に worktree を自動生成するには、一段上の Agent Orchestrator が必要になる。 |
| Chat router | 今回は採用しない | 自然文から workflow を自動選択する必要はまだない。Skill + CLI で十分。 |
| PR/Issue の直接 lifecycle 連携 | 今回は採用しない | 直接 API 連携ではなく、workflow template の操作として表現する。 |
| Workflow marketplace/defaults | 今回は採用しない | まずは Releash curated built-ins に絞る。 |
| External triggers | 今回は採用しない | Slack/GitHub/Telegram などの trigger は surface area が大きく、Agent CLI 依存も重い。 |
| Per-node MCP | 今回は採用しない | 現在の Releash workflow boundary では不要。 |
| Web UI 移植 | 採用しない | UI の考え方だけ借りる。Archon の Web UI を移植しない。 |
| Archon 置換 | 採用しない | Releash の Rust engine を維持する。 |
| Workflow map / DAG 表示 | 後回し | まずは timeline と step detail で十分。loop があるため厳密な DAG 表示は誤解を生みやすい。 |

## 中核モデル

milestone [02] で旧 YAML / 旧 `WorkflowState` JSON / 旧 NDJSON 互換を廃棄し、`Workflow`（template 定義）と `NodeDefinition` を新 schema として直接 YAML deserialize 先に据える。旧 `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` 型は削除済みで、`workflow/normalized.rs`（旧→新の変換層）は新設しない。

未来形モデル各々のフィールド詳細・既存モジュールの future core / compatibility adapter 分類・境界条件は、本文書から派生する north star ドキュメントとして [`workflow-engine-model-boundary.md`](./workflow-engine-model-boundary.md) にまとめている。詳細を参照する場合はそちらを正本とすること。

### Workflow Definition

ユーザーが書く workflow template。新 schema の `Workflow`（`name` / `description` / `builtin` / `nodes`）が YAML deserialize の直接先となる。旧 `steps:` 記法・旧 `mode` 記法は受理されない。

### Node Definition

実行単位。`NodeDefinition` は `node_type` を直接持ち、YAML 上は `type: agent | bash | approval | parallel` で表現される。並列 node の子 node は `ChildNodeDefinition`（top-level 専用フィールド `transition_rules` / `cycle_guard` / `parallel_children` / `aggregate` / `command` / `collect` / `resets_cycle_for` を持たない子専用型）として再帰構造から切り離される。

YAML 上の表現例:

```text
type: agent       -> エージェント駆動の作業単位
type: approval    -> 利用者の承認を必要とする待機単位
type: bash        -> シェル実行に相当する作業単位（command 必須）
type: parallel    -> 子 node 群を並列に走らせ、aggregate で収束する単位
```

### Workflow Run

workflow template を 1 回起動した実行インスタンス。

想定フィールド:

```text
run_id
workflow_name
task
status
worktree_path
current_node_name
trigger_source
started_at
updated_at
completed_at
error_reason
```

初期実装では、既存の `WorkflowState.execution_id` を `run_id` として扱える。

`chat_session_id`（main agent narrator session 紐付け）は [16] Main Agent Mediation 着手時に
改めて命名・追加するフィールドとして本マイルストーン群では保持しない。engine が起動時に独自の
「親 ChatSession」を作って `WorkflowState` の永続化先として流用する経路は撤去済みであり、
WorkflowRun は ChatSession 抽象と独立に識別される（永続化は NDJSON event log + Run Store
metadata で完結し、現在状態は in-memory `WorkflowExecution` で派生キャッシュする）。

### Node Execution

run の中で 1 つの node が実行された結果。

想定フィールド:

```text
run_id
node_name
node_type
status
run_index
session_id
started_at
completed_at
result
structured_output
output_contract
token_usage
error_reason
```

既存の `StepHistoryEntry`、`StepOutput`、`ParallelStepState` は、この shape の多くをすでに持っている。

### Workflow Command

workflow state を変化させる唯一の入口。

例:

```text
StartRun
AbortRun
ApproveNode
RejectNode
SubmitOutput
CompleteNode
FailNode
```

UI button、CLI、remote UI、Agent action は、すべてこの command に落とす。

### Workflow Event

engine が発行する append-only な事実。

例:

```text
RunStarted
NodeStarted
NodeCompleted
ApprovalRequested
ApprovalResolved
OutputSubmitted
ValidationPassed
ValidationFailed
RunCompleted
RunFailed
RunAborted
```

既存の `WorkflowEventLog` は `WorkflowEvent` NDJSON adapter に縮退する。旧 `WorkflowLogEvent` 語彙と旧 NDJSON 在庫の互換は維持せず、リリース時に破棄される前提で扱う。

## 互換性境界

新設計を旧概念に合わせて曲げない。**[02] Normalized Workflow 以降は、互換性境界の責務を縮退させ、旧 schema / 旧 state / 旧 NDJSON 互換は維持しない**。旧概念から新モデルへの adapter 層は、user-authored YAML（新 schema として記述される）の入力経路と built-in YAML 提供経路のみに限定する。

```text
User-authored YAML (新 schema) / built-in YAML / external command
        |
        v
compat adapter（user input の YAML 入口に縮退）
        |
        v
Run / Node / Command / Event model
```

ルール（[02] 以降）:

- 旧 `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` は codebase から削除する。
- 既存 `WorkflowState` JSON / NDJSON event log の在庫はリリース時に破棄される前提を許容する（互換は維持しない）。
- 既存 Tauri command の入口・出口形は本マイルストーン範囲では維持し、後続で `WorkflowCommand` typed 入口に寄せる。
- engine 全体に old/new 分岐を散らさない。
- 旧→新の変換層（`workflow/normalized.rs`）は**新設しない**。`schema.rs` が新 `Workflow` / `NodeDefinition` を YAML deserialize 先として直接保持し、engine もそれを直接消費する。
- `worktree_path` 主語の API は、active `run_id` を解決する互換 wrapper として残す。
- 新 API と CLI は `run_id` を主語にする。

## CLI/API の形

CLI は単なる外部操作口ではない。Agent、UI、remote access、engine をつなぐ typed protocol として扱う。

read-only の初期 command:

```sh
releash workflow list
releash workflow runs
releash workflow status <run-id>
releash workflow logs <run-id>
```

mutation command:

```sh
releash workflow run <workflow-name> "<task>"
releash workflow approve <run-id> --step <step-name> --comment "LGTM"
releash workflow reject <run-id> --step <step-name> --reason "Needs stronger tests"
releash workflow abort <run-id>
```

structured output command:

```sh
releash workflow output submit <run-id> --step <step-name> --type <contract> --json '{"key":"value"}'
releash workflow output submit <run-id> --step <step-name> --type <contract> --file output.json
releash workflow output validate <run-id> --step <step-name> --file output.json
releash workflow output get <run-id> --step <step-name>
```

最初の実装では、CLI は起動中の Releash app を local API 経由で操作する。headless engine は別プロジェクトとして後回しにする。

## UI 方針

右パネルを切り替え可能にする。

```text
Right panel
  - Review
  - Workflow
```

Workflow panel に置くもの:

- workflow run history
- active run summary
- timeline
- step/node detail
- step conversation transcript
- approval actions
- logs and structured output

main agent は user-facing narrator として残す。

- 進捗を報告する
- 完了 step を説明する
- 承認を依頼する
- 失敗 summary を伝える

main agent は state transition を所有しない。approve、reject、abort、output submission は CLI/API command として engine に戻す。

## マイルストーン

### [01] 設計境界の固定

目的: 互換性の圧力で設計が崩れる前に、目標モデルを固定する。

作業:

- 未来形モデルを定義する: `WorkflowRun`, `NodeDefinition`, `NodeExecution`, `WorkflowCommand`, `WorkflowEvent`。
- 互換性をどこに閉じ込めるかを決める。
- この文書を north star として追加する。
- runtime behavior は変えない。

完了条件:

- モデルが文書化されている。
- どの module が compatibility adapter で、どの module が future core か説明できる。

成果物: [`workflow-engine-model-boundary.md`](./workflow-engine-model-boundary.md)（未来形モデル仕様と既存モジュール分類の正本）。

### [02] Normalized Workflow

目的: 旧 `Step` schema を**削除**し、新 `NodeDefinition` を YAML deserialize 先として直接持つ構造に統一する。

方針:

- 旧 `Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode` を削除する（codebase から完全に消す）。
- 新 `Workflow`（template 定義）/ `NodeDefinition` および関連型を `schema.rs` に直接導入する。`workflow/normalized.rs` は新設しない。
- YAML schema を `node_type` ベース（`type: agent | bash | approval | parallel`）で書き直す。`mode` は廃止する。
- 旧 `Step` の各 mode/parallel 表現は、新 schema 上では node_type で直接表現する:

```text
旧 mode: auto / mode 未指定  → 新 type: agent
旧 mode: approval            → 新 type: approval
旧 mode: interactive         → 新 type: agent（対話前提の agent として扱う）
旧 parallel: [...]           → 新 type: parallel + parallel_children
旧 aggregate: ...            → 新 parallel node の aggregate 振る舞い
```

- `built-in/spec-driven-development.yml` を新 schema で書き直す（既存挙動と等価）。
- `state.rs::WorkflowState.workflow_definition` と `event.rs::WorkflowEvent::RunStarted.workflow_definition` の型を新 `Workflow` に揃える（在庫 JSON / 旧 NDJSON は破棄前提）。
- `engine.rs` / `contract.rs` は旧 schema 型を一切 import しない状態にする。
- `validation.rs` / `diagnostics.rs` / `storage.rs` / `facet.rs` / `builtin.rs` / `runtime_view.rs` の compat adapter 群と、`commands.rs` / `agent_commands.rs` / `session_commands.rs` / `session/mod.rs` / `workflow_state_presenter.rs` の caller 群を新型に追従させる。

完了条件:

- 旧 schema 型（`Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）が codebase に存在しない（`grep` で 0 件）。
- `workflow/normalized.rs` は新設されていない。
- `spec-driven-development.yml` が新 schema で書き直され、既存挙動と等価に実行できる。
- node_type（agent / approval / parallel / bash）別の load unit test が `schema.rs` 内に存在する。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` および `pnpm lint` / `pnpm test` / `pnpm build` が成功する。

### [03] Run Store / Run ID

目的: workflow 実行を `run_id` で参照できるようにする。

作業:

- `workflow/run.rs` を追加する。
- `WorkflowRunSummary` を追加する。
- run metadata を保存する。保存先候補は `workflow_runs/{run_id}.json`。
- 既存 `execution_id` を `run_id` として使う。
- active lookup を追加する: `worktree_path -> active run_id`。
- reverse lookup を追加する: `run_id -> worktree_path`。

完了条件:

- active run と completed run を metadata/logs から一覧できる。
- 既存の worktree-scoped state が動き続ける。

### [04] Command / Event Boundary

目的: state change を typed command 経由にする。

作業:

- `WorkflowCommand` を追加する。
- start、approve、reject、abort の command handler を追加する。
- 既存 Tauri command は残しつつ wrapper 化する。
- 既存 `WorkflowLogEvent` を廃止し、`WorkflowEvent` NDJSON へ完全置換する。

完了条件:

- 既存 UI command が動き続ける。
- 新しい internal command path が test されている。
- state transition が main-agent free text に依存しない。

### [05] Read-Only Run APIs + CLI

目的: 外部 caller が workflow run を観測できるようにし、engine 内部の node 完了/失敗遷移も typed command として揃える。

作業:

- Tauri/local API command を追加する:
  - `list_workflow_runs`
  - `get_workflow_run`
  - `get_workflow_run_log`
  - `get_workflow_run_state`
- CLI を追加する:
  - `workflow list`
  - `workflow runs`
  - `workflow status <run-id>`
  - `workflow logs <run-id>`
- engine 内部の typed 遷移 command を追加する:
  - `WorkflowCommand::CompleteNode`
  - `WorkflowCommand::FailNode`
  - これらは外部入口（UI / CLI / Agent）には公開せず、engine 内部の状態遷移を typed に表現するためのもの。NodeCompleted / NodeFailed event の発行点として、観測経路（API / CLI）整備と同じ marker で揃える。

完了条件:

- running workflow を `run_id` で inspect できる。
- completed workflow の log を `run_id` で読める。
- engine 内部の node 完了/失敗遷移が `CompleteNode` / `FailNode` typed command 経由で行われる。

### [06] Mutating CLI

目的: `run_id` を操作の主語にする。

作業:

- CLI/API を追加する:
  - `workflow approve <run-id>`
  - `workflow reject <run-id>`
  - `workflow abort <run-id>`
- 既存 engine path のために内部で `run_id -> worktree_path` を解決する。
- 旧 `worktree_path` command は wrapper として維持する。

完了条件:

- CLI から approve/reject/abort できる。
- 既存 UI approval が動き続ける。
- stale な approval target や unauthorized target が拒否される。

### [07] Workflow Panel / Command Center

目的: UI model を整理する。

作業:

- 右パネルに Review/Workflow switch を追加する。
- Workflow panel に active run と run history を表示する。
- run event timeline を追加する。
- step detail view を追加する。
- step conversation transcript を追加する。
- approval button は CLI と同じ command boundary に通す。

完了条件:

- workflow step が自由対話 chat tab と同格に見えない。
- 多数の chat session を切り替えなくても run を inspect できる。

### [08] OutputForm CLI

目的: agent-to-engine のデータ受け渡しを typed にする。

作業:

- CLI/API を追加する:
  - `workflow output submit`
  - `workflow output validate`
  - `workflow output get`
- 既存 output contract validation を再利用する。
- valid submit 時に `step_outputs` と `workflow_variables` を更新する。
- output submission event を発行する。
- `<workflow_output>` 抽出は fallback として維持する。

完了条件:

- step agent が prose parsing に頼らず structured output を提出できる。
- invalid output が決定論的な validation error になる。

### [13] Node Type + Bash Gate

目的: agent 以外の決定論的実行を追加する。

作業:

- 新 schema は [02] で既に `type: agent | bash | approval | parallel` を持つ。本マイルストーンでは `type: bash` の**実行系統**を engine に実装する（[02] では型・load・schema までの対応で、engine からは bash 開始を明示拒否している）。
- 以下を capture する:
  - command
  - exit code
  - stdout
  - stderr
  - duration
- bash result を structured step output として保存する。
- exit code による branching を support する。

完了条件:

- workflow から `pnpm test` や `cargo test` を validation node として実行できる。
- validation failure を fix node へ route できる。

### [14] Templates

目的: 新しい primitive を実用 workflow にする。

作業:

- curated built-in template を追加する。候補:
  - `validate-and-fix`
  - `smart-review`
  - `refactor-safely`
  - `idea-to-implementation`
- PR/Issue API への直接 coupling は避ける。

完了条件:

- template が必要に応じて bash gate と run management を使う。
- template が YAML として読みやすい。

### [15] Skill

目的: Agent に Releash workflow の使い方を教える。

作業:

- Agent 向け skill/instructions file を追加する。
- 以下を説明する:
  - いつ workflow を使うか
  - workflow list の見方
  - workflow run の起動方法
  - structured output の提出方法
  - user approval を勝手に決めず、どう依頼するか
- command が安定してから文書化する。

完了条件:

- Agent が追加の UI instruction なしに、適切な workflow を CLI で起動できる。

### [16] Main Agent Mediation

目的: main agent に state authority を与えず、自然な workflow 対話を実現する。

作業:

- typed event を発行する:
  - approval requested
  - approval resolved
  - step completed
  - run failed
  - run completed
- それらの event を main agent に渡し、user-facing report に使う。
- user decision は必ず CLI/API command として戻す。

完了条件:

- main agent が承認依頼と結果報告をできる。
- engine が唯一の state transition authority であり続ける。

## 互換性テスト

milestone [02] 以降では、旧 `WorkflowState` JSON / 旧 NDJSON は load 経路から除外され（`workflow_definition` を required 化、deserialize 不能なログは listing/reconstruction の対象外）、新 schema として書き直された built-in YAML の挙動等価のみを担保する。

各マイルストーンで以下を守る。

- 新 `spec-driven-development.yml`（新 schema 表現）の挙動等価（step 数・遷移・並列・aggregate・cycle guard・facet 解決結果）
- 既存 Tauri command の入口・出口（典型 happy path）
- approval/reject branching
- parallel aggregate behavior
- cycle guard behavior
- output contract extraction and repair
- remote workflow state sync

追加したい focused test:

- built-in workflow の normalization snapshot
- active worktree からの `run_id` lookup
- 旧 worktree-scoped command wrapper
- stale approval target rejection
- structured output validation success/failure
- bash node exit-code routing

## 実装メモ

追加候補 module:

```text
src-tauri/src/workflow/run.rs       # [03] WorkflowRun store / run_id 主語管理
src-tauri/src/workflow/command.rs   # [04] WorkflowCommand typed 入口
src-tauri/src/workflow/event.rs     # [04] WorkflowEvent 出口（NDJSON vocabulary 寄せ）
```

`workflow/normalized.rs` は [02] で削除前提に方針変更されたため新設しない。

既存 module は可能な限り現在の責務を保つ。

- `schema.rs`: user-authored workflow schema
- `validation.rs`: workflow validation
- `engine.rs`: execution and state transition authority
- `state.rs`: UI 向け serialized workflow state
- `log.rs`: append-only execution events
- `commands.rs`: Tauri/local API command wrappers

最重要ルール:

```text
Future core は Run / Node / Command / Event を見る。
Compatibility adapter が Step / WorkflowState / worktree_path をその model へ変換する。
```
