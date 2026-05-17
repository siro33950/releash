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

既存 YAML と既存 Tauri command を維持しながら、内部の未来形モデルを明確にする。

未来形モデル各々のフィールド詳細・既存モジュールの future core / compatibility adapter 分類・境界条件は、本文書から派生する north star ドキュメントとして [`workflow-engine-model-boundary.md`](./workflow-engine-model-boundary.md) にまとめている。詳細を参照する場合はそちらを正本とすること。

### Workflow Definition

ユーザーが書く workflow template。既存の `Workflow` と `steps:` YAML は有効なままにする。

### Node Definition

内部実行用に正規化された実行単位。

既存の `Step` は、実行前に `NodeDefinition` へ正規化する。これにより後方互換を engine の中核状態遷移から切り離す。

変換例:

```text
mode: auto          -> type: agent
mode: approval      -> type: approval
parallel: [...]     -> type: parallel
aggregate: ...      -> parallel node の aggregate behavior
type: bash          -> command node
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
chat_session_id
current_node_name
trigger_source
started_at
updated_at
completed_at
error_reason
```

初期実装では、既存の `WorkflowState.execution_id` を `run_id` として扱える。

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

既存の `WorkflowEventLog` は、現在の log 互換性を維持しつつ、この event vocabulary に近づける。

## 互換性境界

新設計を旧概念に合わせて曲げない。旧概念を新モデルへ adapter する。

```text
Old Tauri command / old UI / old YAML
        |
        v
compat adapter
        |
        v
Run / Node / Command / Event model
```

ルール:

- 既存 YAML は有効なままにする。
- 既存 Tauri command は動かし続ける。
- 既存 `WorkflowState` JSON は deserialize できるようにする。
- engine 全体に old/new 分岐を散らさない。
- 旧 `Step` から新 `NodeDefinition` への変換は専用の normalization module に閉じ込める。
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

目的: 既存 `Step` YAML と将来の `NodeDefinition` 実行を分離する。

作業:

- `workflow/normalized.rs` を追加する。
- 既存 `Workflow` / `Step` / `ParallelStep` を normalized node に変換する。
- `type` 未指定は `agent` として扱う。
- `mode: approval` は `approval` に map する。
- `parallel` block は `parallel` node に map する。
- 既存 validation behavior は維持する。

完了条件:

- `spec-driven-development.yml` が挙動変更なしで normalize できる。
- mode/type/parallel 変換の unit test がある。

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
- 既存 `WorkflowLogEvent` と共存できる形で `WorkflowEvent` を追加する。

完了条件:

- 既存 UI command が動き続ける。
- 新しい internal command path が test されている。
- state transition が main-agent free text に依存しない。

### [05] Read-Only Run APIs + CLI

目的: 外部 caller が workflow run を観測できるようにする。

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

完了条件:

- running workflow を `run_id` で inspect できる。
- completed workflow の log を `run_id` で読める。

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

- workflow schema か normalized layer に `type` support を追加する。
- `type: bash` を実装する。
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

各マイルストーンで以下を守る。

- 既存 `spec-driven-development.yml` の挙動
- 既存 `WorkflowState` JSON の deserialization
- 既存 Tauri command
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
src-tauri/src/workflow/normalized.rs
src-tauri/src/workflow/run.rs
src-tauri/src/workflow/command.rs
src-tauri/src/workflow/event.rs
```

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

