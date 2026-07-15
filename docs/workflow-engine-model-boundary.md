# ワークフローエンジンのモデル境界

この文書は、現在の Releash workflow engine で「どのモデルが何を所有するか」と「その責務をどの layer が実装するか」を示す境界図である。将来モデルの提案書ではない。

詳細の一次 Owner は次の文書に分ける。

| 領域 | 正本 |
| --- | --- |
| 正規語彙と状態所有 | [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md) |
| engine の戦略、採用判断、実行モデル | [`workflow-engine-evolution-plan.md`](./workflow-engine-evolution-plan.md) |
| WorkflowDefinition YAML grammar | [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md) |
| Rust layer と依存方向 | [`architecture/README.md`](./architecture/README.md) 以下 |

本文書はそれらを再定義せず、モデル間と実装 layer 間の接続点を固定する。

## モデルの関係

```text
Global / App
  └─ WorkflowDefinition
       └─ NodeDefinition[]

Workspace
  ├─ references Worktree
  ├─ WorkflowExecution
  │    ├─ Artifact[]
  │    └─ NodeExecution[]
  │         └─ fanout_parent ──► parent NodeExecution
  ├─ Session
  ├─ Terminal
  └─ Thread / Comment
```

### WorkflowDefinition / NodeDefinition

`WorkflowDefinition` は管理される workflow template であり、実行状態を持たない。`NodeDefinition` はその内部にだけ存在し、`command` / `session` / `fanout` の kind block、Contract 参照、Artifact 参照、rules を持つ。

YAML は adaptor の schema 型へ直接 deserialize され、domain の `WorkflowDefinition` / `NodeDefinition` へ一対一で写される。別文法へ変換する normalization layer は置かない。grammar と validation の詳細は YAML 構文の正本に従う。

### WorkflowExecution

`WorkflowExecution` は WorkflowDefinition の一回の実行であり、workflow runtime state の主語である。

- `execution_id` で識別する。
- Workspace と対象 Worktree、WorkflowDefinition 名、起動元、現在 Node、時刻、失敗・中断理由を集約する。
- status は `running` / `waiting_approval` / `interrupted` / `completed` / `failed` / `aborted`。
- production の read model は append-only event log の replay から backend が構築する。
- UI / CLI / API は同じ backend-owned read model を読み、独自の workflow state を所有しない。

`interrupted` は終端ではなく resume 可能な checkpoint である。`failed` / `completed` / `aborted` は終端状態。

### NodeExecution

`NodeExecution` は NodeDefinition の一回の実行を表す。`node_execution_id` は WorkflowExecution 内の実行個体を識別し、同じ NodeDefinition が loop や fanout で複数回・並行して実行されても一意である。

NodeExecution は kind、attempt、status、session 参照、検証済み Artifact、token usage、失敗情報、時刻を持つ。fanout child は `fanout_parent` に親 NodeExecution と item / child の位置を記録する。

approve と Artifact 提出は通常 node 名で対象を指定できる。同名 NodeExecution が複数 active な fanout child では `node_execution_id` が対象の曖昧さを解消する。

### Fanout

Fanout は親 NodeExecution と、その `fanout_parent` を指す子 NodeExecution 群から構築する derived view である。別種の child 実行モデルや専用 lifecycle state は持たない。

- child は通常の command / session NodeDefinition。
- 子 Artifact は親 fanout の配列 Artifact に集約され、child 名の global Artifact map には置かない。
- child の rules は fanout 実行中に評価しない。
- resume 時は event log から完了済み child を復元し、未確定 child だけを再実行する。

### Artifact / Contract

Artifact は WorkflowExecution / NodeExecution 間の検証済みデータであり、独立した lifecycle state を持たない。`request` は起動時入力から作る String scalar Artifact、Node の Artifact は NodeExecution の確定結果である。

Contract は `schemas` に宣言する型と validation 規則であり、Artifact と同じく主要 Entity ではない。command、session、外部 Artifact 提出は同じ Contract engine を使う。

### Diagnostic

Diagnostic は WorkflowDefinition の parse / shape / resolve / typecheck / control-flow に対する validation result であり、WorkflowExecution や NodeExecution の status ではない。Rust backend が生成し、UI は code / stage / span / message を表示する。

## 状態変更と観測の境界

```text
UI / CLI / Local API / Agent action
              |
              v
       controller / protocol
              |
              v
       typed command usecase
              |
              v
 Workflow runtime engine ──append──► event log
              |                         |
              └──────── projection ◄────┘
                            |
                            v
             WorkflowExecution / NodeExecution
```

状態変更は usecase の typed command を唯一の入口とする。

- start: WorkflowDefinition 名、Worktree、request、permission mode から WorkflowExecution を始める。
- approve: `gate: approval` で待つ NodeExecution を完了可能にする。
- output submit: 対象 NodeExecution に Contract 検証済み Artifact を提出する。
- stop: active な WorkflowExecution を `interrupted` にする。
- resume: `interrupted` の event log を replay し、最後の確定 checkpoint から再開する。
- abort: active または interrupted な WorkflowExecution を終端にする。

query は projection を読むだけで state を変更しない。Tauri command と local API handler は同じ usecase を呼ぶ薄い controller であり、CLI は local API を通して同じ command boundary に入る。

event log は永続化・projection・観測のための append-only adapter 語彙で、domain entity ではない。現在状態の source of truth を frontend snapshot や個別ファイル reader に分散させない。

runtime 内の `RuntimeCommitSnapshot` は、lock 内で確定した変更を event append、stale commit guard、ExecutionStore 同期、session cleanup、次 action へ原子的に引き渡す private な commit payload である。永続化形式でも公開 read model でもなく、旧 workflow state の互換 reader ではない。live notification 用の `WorkflowRuntimeSnapshot` も同様に一時 projection DTO であり、query の source of truth にはしない。

## 隣接ドメインとの境界

- Workspace は Worktree を参照するが所有しない。Repository / Code / Diff も workflow state ではない。判断材料として固定する場合は Artifact にする。
- Session は Workspace に属する独立モデル。session NodeExecution は `session_id` を参照するが、Session や Turn を WorkflowExecution の内包物にしない。
- Terminal は人間が操作する interactive shell session。workflow の command NodeExecution とは別物である。
- Thread / Comment は Workspace に属し、WorkflowExecution / NodeExecution には属さない。必要な値は外部 command または Artifact を介して workflow が扱う。
- UI / CLI / API は operation surface であり、domain state を所有しない。

## 現行 module Owner

workflow 実装は `src-tauri/src/{domain,usecase,adaptor}/**/workflow/` に分かれる。単一の `src-tauri/src/workflow/` core や compatibility subtree は存在しない。

| 責務 | 現行 Owner |
| --- | --- |
| domain の WorkflowDefinition / NodeDefinition | `src-tauri/src/domain/workflow/value_objects/definition.rs` |
| Contract / Artifact 参照 / routing / validation | `src-tauri/src/domain/workflow/services/contract_schema.rs`、`reference.rs`、`routing.rs`、`validation.rs` |
| WorkflowExecution / NodeExecution read model | `src-tauri/src/domain/workflow/value_objects/execution.rs`、`node_execution.rs` |
| typed mutation command | `src-tauri/src/usecase/workflow/command/` |
| query と外部向け read model | `src-tauri/src/usecase/workflow/query_service.rs`、`dto.rs` |
| YAML schema、span、Diagnostic、definition persistence | `src-tauri/src/adaptor/gateway/workflow/schema.rs`、`span_map.rs`、`diagnostics.rs`、`definition_repository.rs` |
| runtime state、kind 実行、event append | `src-tauri/src/adaptor/gateway/workflow/runtime_engine*.rs` と同ディレクトリの runtime gateway 群 |
| event replay と execution projection | `src-tauri/src/adaptor/gateway/workflow/event.rs`、`event_projection.rs`、execution repository 群 |
| Tauri / local API entry | `src-tauri/src/adaptor/controller/command/workflow/`、`src-tauri/src/adaptor/controller/api/workflow.rs` |
| protocol / presenter | `src-tauri/src/adaptor/protocol/workflow.rs`、`src-tauri/src/adaptor/presenter/workflow.rs` |

依存方向は architecture 規約に従う。domain は外部形式を知らず、usecase は domain port を介して手順を持ち、controller / gateway が外部形式と I/O を担当する。

## 互換性境界

- WorkflowDefinition loader は正本 YAML schema を直接受理し、別 schema からの変換 layer や feature flag を持たない。
- event log と workflow state は現行 event / projection 形式を直接読み、廃止済み形式の reader や変換 adapter を持たない。
- 外部 API は `execution_id` / WorkflowExecution / NodeExecution を主語とする。
- frontend は backend DTO の UI mirror に留まり、validation、routing、resume 判断を実装しない。

## 不変条件

- WorkflowDefinition と WorkflowExecution を混同しない。定義は実行状態を持たない。
- NodeDefinition と NodeExecution を混同しない。定義は WorkflowDefinition、実行個体は WorkflowExecution に属する。
- engine 以外が workflow state transition を決めない。
- Artifact は Contract 検証済みの判断材料であり、lifecycle state を持たない。
- Fanout child も通常の NodeExecution であり、専用 child model を作らない。
- Diagnostic を lifecycle state として保存しない。
- operation surface と frontend mirror を state owner にしない。
