# ワークフローエンジン 設計境界（north star）

本文書は、Releash のワークフローランタイムが将来的に主語として扱うべき未来形モデルの仕様と、既存 Rust モジュールにおける future core / compatibility adapter の境界を確定する north star ドキュメントである。

位置付け・前提・採用方針の戦略記述は [`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md) を一次 Owner とする。本文書では戦略の重複を避け、「何が」「どこに」属するかを定義することに徹する。

## 本文書のスコープ

- 担当する:
  - 未来形 5 モデル（`WorkflowRun` / `NodeDefinition` / `NodeExecution` / `WorkflowCommand` / `WorkflowEvent`）の責務とフィールド定義。
  - 既存 Rust モジュール（`src-tauri/src/workflow/`）の future core / compatibility adapter 分類。
  - マイルストーン [01] で合意した境界条件（互換性境界・state 入口・event 性質）。
- 担当しない:
  - Rust の struct / enum としての型コード化（[02] 以降）。
  - 既存モジュールの再配置・型強制・注釈追加。
  - plan doc の戦略記述（採用判断・マイルストーン進行管理）の重複転載。

## 前提となる互換性境界

本文書で定義する未来形モデルは、[02] Normalized Workflow 以降は**互換性境界の責務を縮退させる**。`docs/workflow-engine-evolution-plan.md` の「互換性境界」節に従い、以下が前提となる。

- 旧 schema 型（`Workflow` / `Step` / `ParallelStep` / `AggregateConfig` / `StepMode`）は [02] で codebase から削除済み。`schema.rs` は新 `Workflow`（`nodes: Vec<NodeDefinition>`）/ `NodeDefinition` を YAML deserialize 先として直接保持する。
- 旧 YAML `steps:` 記法と `mode` フィールドは廃止された。新 YAML は `nodes:` 配下に `type: agent | bash | approval | parallel` で記述する。
- 既存 `WorkflowState` JSON / NDJSON event log の在庫はリリース時に破棄される前提を許容する（互換は維持しない）。
- 既存 Tauri command（`commands.rs` で公開される workflow 操作）の入口・出口形は [02] 範囲では維持される。後続マイルストーンで `WorkflowCommand` typed 入口に寄せる。
- 旧 `Step` から新 `NodeDefinition` への変換層（`workflow/normalized.rs`）は新設しない。schema 層が直接 deserialize 先となる。
- `worktree_path` 主語の API は、active `run_id` を解決する互換 wrapper として残す。
- compat adapter 層は user-authored YAML（新 schema として記述される）の入力経路と built-in YAML 提供経路のみに縮退する。

## 未来形モデル

5 モデルの関係を概念図で示す。

```text
Workflow Definition (新 schema YAML)
        |
        |  serde deserialize → schema::Workflow / NodeDefinition
        |  ＋ validation（schema 構造の静的検証）
        |  ＋ facet 解決（resolved_facets に格納）（load 経路）
        v
NodeDefinition*  ← run の中の各実行単位（解決済み）
        |
        |  StartRun (WorkflowCommand)
        v
WorkflowRun ───────────────► WorkflowEvent*（append-only）
        |
        |  node 実行の都度
        v
NodeExecution*（run に紐づく実行結果）

外部 (UI / CLI / Agent / Remote) ──► WorkflowCommand ──► engine
```

[02] 完了時点では旧 schema は削除済みで、`schema.rs` が新 YAML を直接 deserialize する。
旧→新の変換層（`workflow/normalized.rs`）は新設しない。

それぞれのモデルは、現行コードの対応物が部分的に存在するが、本文書では「将来このように扱う」という宣言にとどめ、現行コードへの注釈や型強制は行わない。

### WorkflowRun

workflow template を 1 回起動した実行インスタンスを識別するモデル。後続マイルストーン以降、UI / CLI / API は worktree ではなく `run_id` を主語にする。

責務:

- 1 回の実行インスタンスを他の実行と区別して識別する（同一 template の複数回実行が並行・継起しうる）。
- 実行の現在状態と関連リソース（worktree、chat session、現在 node）を集約する。
- 実行の進行に応じて状態の遷移と監査可能なタイムスタンプを保持する。

フィールド（暫定）:

- `run_id`: 実行インスタンスを一意に識別する ID。初期実装では既存 `WorkflowState.execution_id` を `run_id` として扱える。
- `workflow_name`: どの workflow template を実行しているか。
- `task`: 実行起動時に渡された task 表現（自由テキスト相当）。
- `status`: 実行のライフサイクル状態（pending / running / awaiting_approval / completed / failed / aborted など）。最終的な状態語彙は [03] / [04] で確定する。
- `worktree_path`: 実行が紐づく worktree。互換 wrapper 経由の lookup の起点でもある。
- `current_node_name`: 現在処理中（または直近停止した）NodeDefinition の名前。

  なお `chat_session_id`（main agent narrator session 紐付け）は [16] Main Agent Mediation 着手時に
  改めて命名・追加するフィールドとして本マイルストーン群では保持しない。engine が起動時に独自の
  「親 ChatSession」を作って `WorkflowState` の永続化先として流用する経路は撤去済みであり、
  WorkflowRun は ChatSession 抽象と独立に識別される。
- `trigger_source`: 起動経路（UI / CLI / remote / agent など）。
- `started_at` / `updated_at` / `completed_at`: 状態遷移を辿れる時刻情報。
- `error_reason`: failure / abort 時の理由。

他モデルとの関係:

- `WorkflowEvent` の発行主体に紐づく一次 ID は `run_id`。
- `NodeExecution` は必ず 1 つの `WorkflowRun` に属する（`run_id` を保持する）。
- `WorkflowCommand` の多くは対象 run を `run_id` で指す。

### NodeDefinition

run の中で扱われる実行単位。[02] では新 schema YAML が直接 deserialize 先となり、engine は `NodeDefinition` のみを参照する（旧 `Step` / `ParallelStep` / `AggregateConfig` は削除済み）。

責務:

- run 中の各実行単位を「種別 + 振る舞いに必要な設定」として均一に表現する。
- 新 YAML 記法（`type: agent | bash | approval | parallel`）を直接 deserialize 先として保持する。
- node 種別ごとの実行戦略の入口を一本化する。

フィールド（暫定）:

- `node_name`: 当該実行単位の名前。run 内で識別子として用いる。
- `node_type`: `agent` / `bash` / `approval` / `parallel` の 4 種の実行種別（aggregate は parallel node の収束設定として `parallel_children` 側に集約される）。[13] で `bash` の取り扱いが具体化する。
- `agent_config`: type=agent 系の振る舞い設定（policy / knowledge / instruction / output_contract、pass_previous_response、pass_output_from、inline_prompt、collect 等）。
- `command_config`: type=bash 系の command 表現（command 文字列、exit code 取り扱い方針など）。
- `approval_config`: type=approval 系の必要承認条件・承認後遷移ルール。
- `parallel_children`: type=parallel 時に並列実行する子 node 群（`ChildNodeDefinition`、子専用型で top-level 専用フィールドを構造的に持たない）と aggregate 戦略。
- `resolved_facets`: load 経路で facet ref（policy / knowledge / instruction / output_contract）から解決した本文キャッシュ（serialize 対象外）。実行時に engine が直接参照し、未解決 ref を schema 層に残さない。
- `transition_rules`: 完了結果に応じた次 node 解決ルール（既存 `TransitionRule` 相当）。
- `cycle_guard` / `resets_cycle_for`: 既存サイクルガード意味論をそのまま受け継ぐ。
- `model` / `permission` などの実行コンテキスト override。

[02] では旧 `Step` schema が削除されたため、変換ではなく**新 schema の構文として直接 `node_type` を表現する**:

```text
新 YAML 構文（[02] Normalized Workflow 以降）

nodes:
  - name: ...
    type: agent          # 旧 mode: auto / interactive / 未指定 はすべて agent に統合
    instruction: ...
    policy: ...
  - name: ...
    type: approval       # 旧 mode: approval
    instruction: ...
  - name: ...
    type: bash           # 新規。実行系統は [13] で具体化
    command: ...
  - name: ...
    type: parallel       # 旧 parallel: [...] block
    parallel_children:   # ChildNodeDefinition の列。再帰構造は持たない（DoS 防御）
      - name: ...
        type: agent
        # 子 node は transition_rules / cycle_guard / parallel_children / aggregate /
        # command / collect / resets_cycle_for を型レベルで持てない。
      ...
    aggregate:           # 旧 aggregate: ...
      all_match: LGTM
      then: ...
      else: ...
```

旧 schema からの YAML マイグレーションは利用者が手で行う前提とする（自動マイグレーションツールは [02] では作らない）。

他モデルとの関係:

- `WorkflowRun` の進行中、現在 node は NodeDefinition で指される。
- `NodeExecution` は 1 つの NodeDefinition の 1 回分の実行を表す（同じ NodeDefinition が `run_index` 違いで複数回実行されることを許容する）。

### NodeExecution

WorkflowRun の中で 1 つの NodeDefinition が実行された結果。状態遷移の足跡として、append-only に積まれる NodeExecution 列と、`WorkflowEvent` 列とで監査性を担保する。

責務:

- ある実行単位の 1 回分の処理結果を「所属 run + node + 反復回」で識別可能に記録する。
- 構造化出力・契約検証結果・トークン使用量・失敗理由など、後続 UI / CLI / Skill が観測する観測単位を保持する。
- 既存 `StepHistoryEntry` / `StepOutput` / `ParallelStepState` が部分的に担っている情報を、`run_id` 主語で一元化する場として位置付ける。

フィールド（暫定）:

- `run_id`: 所属する WorkflowRun。
- `node_name`: 実行された NodeDefinition の名前。
- `node_type`: 実行時点での node 種別（NodeDefinition と整合）。
- `status`: 当該実行のライフサイクル状態（pending / running / awaiting_approval / succeeded / failed / aborted など）。
- `run_index`: 同一 node の何回目の実行か（cycle / 再試行を識別）。
- `session_id`: 当該実行に対応する session（agent 実行時など、存在する場合）。
- `started_at` / `completed_at`: 実行のタイムスタンプ。
- `result`: 実行結果の自由テキスト要約（互換用途）。
- `structured_output`: 構造化出力。output_contract に従って検証済みの場合は contract の type 名と JSON を保持する。
- `output_contract`: 検証に用いた contract 種別の識別子。
- `token_usage`: 当該実行のトークン使用量。
- `error_reason`: failure / abort 時の理由。
- `child_executions`: parallel node の場合、子 NodeExecution（または子の参照）を集約する。

他モデルとの関係:

- WorkflowRun:NodeExecution = 1:N。
- NodeDefinition:NodeExecution = 1:N（同一 node の複数回実行を許容）。
- 主要な状態変化は `WorkflowEvent` として併発的に発行され、NodeExecution と event log の両方から実行履歴を辿れる。

### WorkflowCommand

workflow state を変化させる唯一の入口。UI button、CLI、remote UI、Agent action は、すべてこの command に落として engine に到達する。`run_id` を主語とする。

責務:

- engine の状態遷移エンドポイントを typed に集約する（main agent の自由文や implicit な副作用で state が動くことを禁ずる）。
- 旧 Tauri command や旧 worktree 主語 API は、command への変換器（compat adapter）を経由してこの入口に揃える。
- 認可・冪等性・stale target の判定など、state 入口の前段で要求される検査の起点となる。

主要 command（暫定）:

- `StartRun { workflow_name, task, worktree_path, trigger_source }`: 新規 run の起動。
- `AbortRun { run_id, reason? }`: 進行中 run の中断。
- `ApproveNode { run_id, node_name, comment? }`: approval node に対する承認。
- `RejectNode { run_id, node_name, reason }`: approval node に対する却下。
- `SubmitOutput { run_id, node_name, contract_type, payload }`: 構造化出力の提出。
- `CompleteNode { run_id, node_name, ... }`: node 完了の通知（内部用途中心。マイルストーン [05] で導入）。
- `FailNode { run_id, node_name, reason }`: node 失敗の通知（内部用途中心。マイルストーン [05] で導入）。

`CompleteNode` / `FailNode` はマイルストーン [04] では導入せず、観測経路（API / CLI）整備と同じマイルストーン [05] で typed 化される。マイルストーン [04] の対象は外部入口を伴う 4 command（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode`）に限定される。

他モデルとの関係:

- 受理された command は engine 内で state 遷移を発生させ、結果として 1 つ以上の `WorkflowEvent` が発行される。
- 命令の到達経路（UI / CLI / Agent / Remote）に依らず、同一 command であれば engine からは等価に扱われる。
- 旧 API（worktree_path 主語）は active `run_id` を解決して command に変換する compat adapter として扱われる。

### WorkflowEvent

engine が発行する append-only な事実列。`WorkflowCommand` の処理結果として（あるいは engine 内部の遷移の都度）発行され、過去の event は書き換わらない。

責務:

- engine の状態遷移を観測可能な事実として外部に提示する（UI timeline、CLI logs、main agent narrator、remote sync が共通購読する語彙）。
- 既存 `WorkflowLogEvent` 語彙を置換し、`WorkflowEvent` の NDJSON 列を唯一の event log vocabulary として扱う。旧 NDJSON 在庫は破棄前提で、互換 reader は持たない。
- 「事実」と「現在状態」を分離する: 現在状態は `WorkflowRun` / `NodeExecution` から読み、履歴は event 列から辿る。

主要 event（暫定）:

- `RunStarted { run_id, workflow_name, task, worktree_path, started_at }`
- `NodeStarted { run_id, node_name, node_type, run_index, started_at }`
- `NodeCompleted { run_id, node_name, run_index, status, result?, structured_output?, completed_at }`
- `ApprovalRequested { run_id, node_name, requested_at }`
- `ApprovalResolved { run_id, node_name, decision, comment?, resolved_at }`
- `OutputSubmitted { run_id, node_name, contract_type, payload_summary, submitted_at }`
- `ValidationPassed { run_id, node_name, contract_type, validated_at }`
- `ValidationFailed { run_id, node_name, contract_type, reason, validated_at }`
- `RunCompleted { run_id, completed_at }`
- `RunFailed { run_id, reason, failed_at }`
- `RunAborted { run_id, reason?, aborted_at }`

不変条件:

- event は append-only。発行済み event の内容は書き換えない。撤回も新たな event（補正 event / abort 等）として表現する。
- 同じ事実を別経路で二度発行しない（例: command 経由の遷移と内部遷移で重複発行しない）ことを engine の責務とする。

他モデルとの関係:

- 1 件の `WorkflowCommand` 受理は 0 件以上の `WorkflowEvent` 発行に対応する（command 拒否時は 0 件もありうる）。
- `WorkflowRun` / `NodeExecution` の現在状態は、論理的には event 列のリプレイから導けるが、実装上は派生キャッシュとして保持してよい。

## 既存モジュールの分類（future core / compatibility adapter）

`src-tauri/src/workflow/` 配下の既存モジュールを、本マイルストーン時点の運用観点で future core / compatibility adapter のいずれに位置付けるかを示す。本マイルストーンではコードに注釈を入れず、本文書を一次 Owner とする。

分類の語彙:

- **future core**: 未来形 5 モデル（Run / Node / Command / Event）に直接寄せていく中核責務。将来は新モデルに対して語る形にリファクタリングされる。
- **compatibility adapter**: 旧概念（`WorkflowState` / `worktree_path`）と新 schema の入力経路（user-authored YAML / built-in YAML）を未来形モデルへ橋渡しする責務。一次責務が旧概念側または YAML 入口にあるものはここに分類する。[02] 完了後、旧 `Step` / 旧 YAML / 旧 JSON 互換は維持されない。

### 分類表

| モジュール | 分類 | 担当する責務（現状） | 未来形モデルとの関係 |
| --- | --- | --- | --- |
| `schema.rs` | future core 寄り（YAML 入口） | 新 `Workflow`（template 定義）/ `NodeDefinition` の YAML スキーマ定義。`type: agent | bash | approval | parallel` で node 種別を表現する。 | `NodeDefinition` を直接 YAML deserialize 先として保持する。旧 schema 型は [02] で削除済み。 |
| `validation.rs` | compatibility adapter | 新 `Workflow` スキーマの静的検証（名前重複、未知 next、facet 参照キーの形式、parallel 子の制約など）。`facet` 本文（解決済み内容）は参照しない。本文の解決失敗は `facet.rs` 側で扱う。 | YAML レイヤーの検証は引き続き必要。`NodeDefinition` ベースで検証する。 |
| `engine.rs` | future core | 状態遷移の権威。step 実行、approval、parallel/aggregate、cycle guard、contract 検証の中枢。 | `WorkflowCommand` を入口、`WorkflowEvent` を出口とする shape を担う。 |
| `state.rs` | compatibility adapter | UI 向け `WorkflowState`、`StepHistoryEntry`、`StepOutput`、`ParallelStepState`、`TokenUsage` などのシリアライズ shape。 | 多くのフィールドは `NodeExecution` の前身。互換 deserialize を維持しつつ、新モデルへ段階的に投影する対象。 |
| `log.rs` | future core（永続化 adapter） | append-only な `WorkflowEventLog` の NDJSON 永続化。 | `WorkflowEvent` をそのまま append/read する。旧 `WorkflowLogEvent` / 旧 NDJSON 在庫との互換は持たない。 |
| `commands.rs` | compatibility adapter | Tauri / local API command の wrapper 群。UI / Remote の現行操作口。 | 旧 `worktree_path` 主語の入口を `WorkflowCommand` へ変換する adapter として残る。新 CLI / API も最終的にここから `WorkflowCommand` を介する。 |
| `storage.rs` | compatibility adapter | workflow YAML 定義の保存・読み込み・ビルトイン保護。 | 新 `Workflow` / `NodeDefinition` の YAML 入口側の永続化を担い、future core からも参照されうるが一次責務は YAML 入口側にある。run / event の永続化は別レイヤー（[03] 以降）。 |
| `contract.rs` | future core | `<workflow_output>` の抽出と output contract の検証・repair prompt 生成。 | `WorkflowCommand::SubmitOutput` 検証と `OutputSubmitted` / `ValidationPassed` / `ValidationFailed` event の中核ロジック。 |
| `facet.rs` | compatibility adapter | policy / knowledge / instruction / output_contract のファセット解決。 | 新 `NodeDefinition` / `ChildNodeDefinition` の facet 参照解決を一次責務として担い、load 経路で `resolved_facets` に解決結果を流し込む。一次入口は YAML 入口側にある。 |
| `runtime_view.rs` | compatibility adapter | `WorkflowState` 上から step 関連 session id を集約するビュー処理。 | UI / session 連携の互換補助。長期的には `NodeExecution` ベースのビューに置き換わる候補。 |
| `diagnostics.rs` | compatibility adapter | YAML レイヤーの追加診断（facet 欠落、参照不一致など）。 | `validation.rs` と同様に YAML 入口側の責務。 |
| `session_errors.rs` | compatibility adapter | workflow 由来エラーの redaction ヘルパ。 | 旧 `WorkflowState` / 旧 session 連携で生じるエラー文言の整形を一次責務とし、future core 側からも再利用されうるが入口は旧概念側にある。 |
| `builtin.rs` / `builtin/` / `builtin_facets/` | compatibility adapter | curated built-in workflow / facet の同梱。 | 新 schema YAML 入口の curated assets として機能する。将来 [14] で template 追加が進んでも本モジュール群の役割（新 `Workflow` / `NodeDefinition` レイヤーへの同梱・提供）は変わらない。 |

### 補足: ランタイム振る舞いとの境界

本分類はあくまで「将来どちらの責務へ寄せるか」の宣言である。本マイルストーン期間中は、上記いずれのモジュールにも振る舞い変更を加えない。コード上にも分類メタ情報を持たない（注釈追加もしない）。runtime 振る舞いの一次 Owner は引き続き既存 Rust モジュール群（コード）であり、本文書は将来形の宣言として独立に維持される。

## 設計境界の不変条件（マイルストーン [01] 合意）

本マイルストーンで以下を境界として固定する。後続マイルストーンで実装に踏み込む際は、これらを前提とする。

- state を変化させる入口は `WorkflowCommand` に一本化される。旧 Tauri command や旧 worktree 主語 API も、最終的に compat adapter を通じて `WorkflowCommand` に変換されて engine に到達する。
- engine が発行する事実は `WorkflowEvent` の append-only な事実列として積み上がる。発行済み event は書き換わらず、撤回や補正も追加 event として表現される。
- 実行の単位は `WorkflowRun` として識別される。worktree 主語の参照は active `run_id` 解決を経て扱われる。
- workflow template の実行単位は、engine 内部では `NodeDefinition` で扱う。YAML 記述ゆれは `schema.rs` の deserialize で吸収する（正規化レイヤー `workflow/normalized.rs` は新設しない）。
- 1 回の node 実行結果は `NodeExecution` として記録され、所属 `WorkflowRun` および対応する `NodeDefinition` に紐づく。
- 旧 YAML / 旧 `WorkflowState` JSON / 旧 NDJSON event log の互換性は [02] で破棄された。Tauri command の入口・出口形は本マイルストーン範囲では維持する。新設計は旧概念に合わせて曲げず、旧 schema 廃止後は新 schema が一次入口となる。

## 関連文書

- 戦略・採用判断・マイルストーン進行: [`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md)
- 既存実装の現状根拠（runtime 振る舞いの一次 Owner）: `src-tauri/src/workflow/` 配下の Rust モジュール群。
