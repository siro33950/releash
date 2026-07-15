# Releash ドメインモデル Current State

最終更新: 2026-07-15

## 位置付け

この文書は現行実装のモデル配置と状態所有を俯瞰する index である。正規語の定義や旧実装名の対応表ではない。

- 正規語彙、使用禁止語、状態所有: [`../architecture/GLOSSARY.md`](../architecture/GLOSSARY.md)
- workflow model / layer の境界: [`../workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md)
- workflow engine の戦略と実行モデル: [`../workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md)
- WorkflowDefinition YAML grammar: [`../workflow-yaml-syntax.md`](../workflow-yaml-syntax.md)
- Rust layer の責務: [`../architecture/README.md`](../architecture/README.md) 以下

語彙や責務が本文書と上記正本で食い違う場合は、正本を優先する。

## 現在の構造

```text
Global / App
  └─ WorkflowDefinition
       └─ NodeDefinition[]

Workspace
  ├─ references Worktree
  ├─ WorkflowExecution
  │    ├─ Artifact[]
  │    └─ NodeExecution[]
  │         └─ Fanout（親と子 NodeExecution の束ね）
  ├─ Session
  │    ├─ Turn
  │    │    └─ PermissionRequest
  │    └─ Message
  │         └─ MessagePart / Attachment
  ├─ Terminal
  ├─ Thread
  │    └─ Comment
  └─ WorkspaceState

Repository
  └─ Worktree
       ├─ Code
       │    └─ CodeAnchor
       └─ Diff

Operation Surface
  ├─ UI
  ├─ CLI
  └─ API
```

## 状態所有

| モデル | 現在の扱い |
| --- | --- |
| WorkflowDefinition | Global / App に属する管理対象。NodeDefinition と Contract 宣言を持つが、実行状態は持たない。 |
| WorkflowExecution | Workspace に属する workflow runtime state の主語。backend の event replay / projection が read model を所有する。 |
| NodeExecution | WorkflowExecution に属する一回の Node 実行。`node_execution_id` で loop / fanout の実行個体を識別する。 |
| Fanout | 親 NodeExecution と `fanout_parent` を持つ子 NodeExecution 群から構築する derived view。 |
| Artifact | WorkflowExecution / NodeExecution 間の Contract 検証済みデータ。独立 lifecycle state は持たない。 |
| Diagnostic | WorkflowDefinition の validation result。WorkflowExecution / NodeExecution の status ではない。 |
| Workspace | Releash の作業 context。Worktree を参照するが所有しない。 |
| Session / Turn / Message | Agent との対話・実行 context。WorkflowExecution とは別に Workspace に属する。 |
| PermissionRequest | Turn に属する個別の許可要求。workflow approval gate とは別のモデル。 |
| Terminal | 人間が操作する interactive shell session。workflow command Node とは別のモデル。 |
| Thread / Comment | Workspace の会話・判断記録。WorkflowExecution / NodeExecution の子ではない。 |
| WorkspaceState | Workspace の editor tabs / layout 等の UI state。domain behavior を持たない。 |
| Repository / Worktree / Code / Diff | 外部 repository 側の実体または派生 view。Releash-owned state にしない。 |
| UI / CLI / API | operation surface。domain state を所有せず、backend usecase を呼ぶ。 |

Releash core に Task Entity や global task input は置かない。task 的な値が必要な workflow はユーザー定義 Artifact field として表現する。

## Workflow domain の実装状態

milestone 82 の新モデル移行は完了している。

- NodeDefinition は `command` / `session` / `fanout` の kind block をちょうど一つ持つ。
- Contract / Artifact は WorkflowDefinition の `schemas` と Node の `artifact` / `input` / `inputs` に統一されている。
- Artifact 参照は `request` / Node / Node field / `item` / item field に閉じている。
- routing は `when` / `switch` / `next` / `loop_guard` の正規形を Rust が検証する。
- command は標準結果と stdout-JSON Contract field を一つの Artifact 名前空間に持つ。
- session の完了 gate は `auto` / `approval`。Artifact を宣言した session は検証済み提出まで完了しない。
- fanout child も通常の NodeExecution で、空 items と partial resume を扱う。
- WorkflowExecution / NodeExecution / Artifact / Fanout は append-only event log の replay から backend read model として構築される。
- start / approve / Artifact submit / abort / stop / resume は typed command usecase に集約される。
- Tauri / local API / CLI / UI は同じ usecase と backend-owned state を使う。
- YAML、event log、workflow state の廃止形式を読む compatibility layer は持たない。

## 現行 module map

| bounded context / 責務 | 主な場所 |
| --- | --- |
| workflow domain model | `src-tauri/src/domain/workflow/` |
| workflow typed command / query | `src-tauri/src/usecase/workflow/` |
| workflow YAML / persistence / runtime / projection | `src-tauri/src/adaptor/gateway/workflow/` |
| workflow Tauri / local API entry | `src-tauri/src/adaptor/controller/command/workflow/`、`src-tauri/src/adaptor/controller/api/workflow.rs` |
| workflow protocol / presenter | `src-tauri/src/adaptor/protocol/workflow.rs`、`src-tauri/src/adaptor/presenter/workflow.rs` |
| agent session | `src-tauri/src/domain/agent_session/`、`src-tauri/src/usecase/agent_session/`、対応 adaptor |
| workspace UI state | `src-tauri/src/domain/workspace_state/`、対応 usecase / adaptor |
| repository / code | `src-tauri/src/domain/repository/`、`src-tauri/src/domain/code/`、対応 usecase / adaptor |
| comment | `src-tauri/src/domain/comment/`、対応 usecase / adaptor |
| terminal backend | `src-tauri/src/domain/pty_session/`、対応 usecase / adaptor |

実装 package 名が integration や infrastructure の都合を表す場合でも、product / domain の説明と外部 API では GLOSSARY 正規語を使う。

## 境界上の不変条件

- 定義と実行を分ける。WorkflowDefinition / NodeDefinition は runtime state を持たない。
- workflow state transition は engine だけが決める。
- frontend は backend read model の UI mirror に留まり、validation、routing、resume 判断を持たない。
- Session、Terminal、Thread を WorkflowExecution の所有物にしない。必要な参照だけを保持する。
- Worktree / Repository / Code / Diff を Releash-owned state にしない。固定した判断材料は Artifact にする。
- Diagnostic を lifecycle state として永続化しない。
- external integration のモデルを core Entity に昇格させない。
