# Workflow Engine モデル境界

この文書は workflow engine の構造、状態所有、外部境界を定義する。定義構文は [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md)、ライフサイクルは [`../specs/workflow-lifecycle/workflow-ideal-lifecycle.md`](../specs/workflow-lifecycle/workflow-ideal-lifecycle.md) を正とする。

## 構造

```text
Global / App
  └─ WorkflowDefinition
       └─ NodeDefinition

Workspace
  └─ Worktree
       ├─ execution tree
       │    └─ NodeExecution
       │         ├─ Session
       │         ├─ Command
       │         ├─ Fanout
       │         │    └─ child NodeExecution[]
       │         └─ Sequence
       │              └─ child NodeExecution[]
       └─ execution tree
            └─ Session
```

Workspace は Worktree を参照する作業コンテキストであり、Worktree と同一ではない。Worktree 配下には複数の実行木が属する。workflow の実行木も単独 Session の1 Node 実行木も同じ構造であり、Workspace 直下に異なる実行系統として並べない。

WorkflowDefinition は Global / App に属する。NodeDefinition は4種の Node の定義であり、実行状態を持たない。実行開始時に NodeExecution が生まれ、実際に開始したインスタンスだけが再帰的な実行木を構成する。

## 定義と実行の対応

| 定義 | 実行 |
| --- | --- |
| WorkflowDefinition | WorkflowExecution が所有する実行木 |
| NodeDefinition | NodeExecution |
| Session | provider CLI と対話する葉 |
| Command | process を実行する葉 |
| Fanout | 並列 children を束ねる branch |
| Sequence | 時系列 children を束ねる branch |
| children の rule / 隣接辺 | 確定 Artifact から選ばれた実行辺 |

定義は候補を持つが、実行木は起きたことだけを持つ。未開始 Node や選ばれなかった分岐を定義から再生成しない。loop と retry は開始ごとに別 NodeExecution になる。retry は明示的な関係で識別し、同名というだけではまとめない。

## 状態所有

### 実行木全体

WorkflowExecution の全体 status は `Running` / `Completed` / `Aborted` の3値である。

- `Running`: 継続可能、または Node 単位の判断・復旧・再開を待つ。
- `Completed`: root Node の completion が成立した。
- `Aborted`: 人間の abort で木全体が終端した。

### NodeExecution

NodeExecution は `Running`、`WaitingApproval`、`Paused`、`Failed`、`Completed`、`Aborted`、中断理由、completion signal、attempt、Artifact、親 scope を所有する。WaitingApproval、Paused、Failed は木全体の status ではない。

### Session と provider

Session NodeExecution は AgentSession の opaque identity を参照する。AgentSession は provider session identity、transcript reference、provider lifecycle、Terminal ownership を持つ。conversation 本文と provider UI は provider CLI / transcript が所有する。NodeExecution は completion に必要な Submit と provider Stop の観測を持つ。

### Artifact

Artifact は確定した判断材料であり lifecycle state を持たない。NodeExecution と合成子 scope が参照を所有し、UI は Artifact を state transition の代替 source of truth にしない。

### Worktree

通常の Worktree / Repository / Code / Diff は外部 repository 側の実体であり、Releash が内容を所有しない。通常の実行木は root Worktree を参照する。

隔離実行では Node attempt が、生成した隔離 worktree の identity、作成・喪失・掃除候補などの Releash 側 lifecycle state を所有する。隔離 worktree の Git 内容そのものは引き続き外部実体である。隔離成果の統合は engine が自動決定せず、判断主体が Artifact と Diff を確認して行う。

## completion と辺の境界

completion は NodeDefinition / NodeExecution が所有し、何をもって完了とするかを決める。辺は Sequence の children エントリが所有し、完了後にどこへ進むかを決める。Fanout は children の並列展開と全 child の決着を所有する。

Session の completion は Submit と provider Stop の二信号を同じ attempt に対して集約する。`completion: approval` は本来の完了条件の後に human checkpoint を置く。承認は辺ではない。

## command と read model の境界

```text
UI / CLI / API
      │ typed command / query
      ▼
usecase
      │
      ▼
workflow aggregate / workspace tree domain
      │ durable fact
      ▼
event store ── projection ──► shared read model
```

- aggregate だけが transition の受理と次状態を決める。
- usecase は command の編成、repository、外部 effect の順序を扱う。
- adaptor/controller は入力を型へ変換し、domain behavior を持たない。
- Workspace tree domain は開始済み Node、親子関係、順序、status、capability、retry 履歴を投影する。
- Tauri、WebSocket、CLI、将来 client は同じ read model を読む。
- frontend は read model の mirror と UI の開閉・選択だけを持つ。

## 復旧境界

外部 effect の成否を推測しない。Node の canonical fact、provider lifecycle fact、未解決 obligation を突き合わせ、execution ごとに復旧する。解決不能な owner がある場合は Node の recovery reason と capability に反映し、別 execution の観測・復旧を止めない。

## 境界外

- workspace 横断の統合監督 view。
- 定義を跨ぐ runtime 参照。
- Artifact の意味を engine が解釈すること。
- 隔離成果の無条件 merge。
- UI が status や capability を再導出すること。
