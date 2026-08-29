# Releash GLOSSARY

## 目的

Releash のドメイン横断ユビキタス言語を定義する。この文書は設計、実装、レビューで参照する canonical vocabulary である。

## 正規語

| 正規語 | 定義 | 所有者 / 所属 |
| --- | --- | --- |
| WorkflowDefinition | workflow の定義。NodeDefinition、Contract、配線、辺を持つ | workflow / Global |
| WorkflowExecution | WorkflowDefinition から開始された1本の実行木 | workflow / Worktree |
| NodeDefinition | Node の Interface、kind 固有設定、completion の定義 | workflow / WorkflowDefinition |
| NodeExecution | NodeDefinition または単独 Session の一回の実行インスタンス | workflow / execution tree |
| Session | provider CLI と継続対話する葉 Node | workflow |
| Command | 非対話 command を一度実行する葉 Node | workflow |
| Fanout | children を並列に束ねる合成 Node | workflow |
| Sequence | children を時系列に束ね、辺を所有する合成 Node | workflow |
| completion | Node 自身が持つ完了の定義 | NodeDefinition / NodeExecution |
| 実行木（execution tree） | 実際に開始した NodeExecution が作る再帰木 | Worktree |
| 辺（edge） | Node completion 後の進行先。Sequence の children エントリが所有する | Sequence |
| Artifact | NodeExecution 間で生成・参照される確定した判断材料 | workflow |
| Contract | input / Artifact を検証する名前付き schema | WorkflowDefinition |
| Facet | Session の prompt 構成に使う再利用可能な補助部品 | workflow |
| Diagnostic | 定義の構文・参照・型・control-flow の検証結果 | workflow definition loader |
| Workspace | Releash の作業コンテキスト。Worktree を参照するが同一ではない | workspace_state |
| WorkspaceState | editor tabs、layout、選択など Workspace の UI state | workspace_state |
| Worktree | Repository の特定 checkout / working tree | repository |
| 隔離 worktree | `isolated` 宣言で Node attempt ごとに作られる実行環境 | workflow / repository boundary |
| Repository | Worktree の背後にある履歴、remote、branch のまとまり | repository |
| Code | Worktree 内のファイル内容 | code / external state |
| Diff | Worktree / Repository の状態から計算される差分 | code / derived view |
| CodeAnchor | Code 上の位置への参照 | code |
| AgentSession | Session Node が参照する provider CLI の継続 identity、lifecycle、および agent の活動状態 | agent_session |
| ProviderLifecycle | provider session identity、transcript reference、Stop 観測の境界 | provider_lifecycle |
| ProviderAvailability | provider executable と利用可否の境界 | provider_availability |
| Terminal | 人間または AgentSession が操作する interactive shell surface | terminal_surface |
| Thread | Workspace に属する会話・判断履歴 | comment |
| Comment | Thread 配下の一つの記録 | comment |
| UI / CLI / API | domain を操作・観測する surface | operation_surface |

Session は Node の正規語、AgentSession は provider CLI identity を扱う実装境界の正規語であり、同じ概念ではない。NodeExecution は AgentSession を参照できるが、conversation 本文を所有しない。

## 構造

### Global / App

```text
Global / App
  └─ WorkflowDefinition
       ├─ NodeDefinition
       └─ Contract
```

WorkflowDefinition は Global / App に属し、実行状態を持たない。NodeDefinition は WorkflowDefinition に属し、standalone では存在しない。

### Workspace と Worktree

```text
Workspace
  ├─ targets Worktree
  │    ├─ execution tree
  │    │    └─ NodeExecution
  │    │         ├─ Session ──references──► AgentSession
  │    │         ├─ Command
  │    │         ├─ Fanout
  │    │         │    └─ child NodeExecution[]
  │    │         └─ Sequence
  │    │              └─ child NodeExecution[]
  │    └─ execution tree
  │         └─ Session
  ├─ Terminal
  ├─ Thread
  │    └─ Comment
  └─ WorkspaceState
```

- Worktree 配下に、workflow と単独 Session を含む実行木が同じ再帰構造で属する。
- 単独 Session は Session Node 1個を root とする実行木である。
- 実行木には実際に開始した NodeExecution だけが載る。
- Artifact は NodeExecution と合成子 scope から参照されるが lifecycle state を持たない。
- Thread は CodeAnchor を参照できるが、WorkflowExecution / NodeExecution には属さない。

### Repository / Code

```text
Repository
  └─ Worktree
       ├─ Code
       │    └─ CodeAnchor
       └─ Diff
```

Repository、通常の Worktree、Code、Diff は外部 repository 側の実体または派生 view であり、Releash が内容を所有しない。固定した判断材料として保持する場合は Artifact にする。

### Operation Surface

```text
Operation Surface
  ├─ UI
  ├─ CLI
  └─ API
```

Operation Surface は domain state を所有しない。同じ backend usecase と read model を利用する。

## 状態所有

### 実行木

WorkflowExecution は木全体の `Running` / `Completed` / `Aborted` を所有する。WaitingApproval、Paused、Failed、Interrupted と completion signal は NodeExecution が所有する。workflow aggregate だけが transition を決める。

### AgentSession

AgentSession は provider、provider session identity、opaque transcript reference、Terminal ownership、open / paused / archived lifecycle、および agent の活動状態を持つ。活動状態は `Working` / `AwaitingAnswer` / `AwaitingInstruction` の3値であり、lifecycle とは独立して AgentSession が所有する。NodeExecution / WorkflowExecution はこの活動状態を持たない。conversation 本文と provider UI は provider CLI / transcript が所有する。単独 Session の lifecycle も実行木から分離した別の作業モデルにはしない。

### 隔離 worktree

`isolated` 宣言で作られる隔離 worktree では、Node attempt が次の Releash 側状態を所有する。

- root Worktree と owner NodeExecution の identity。
- attempt ごとの隔離 branch / path identity。
- 作成済み、喪失、cleanup candidate などの lifecycle fact。
- recovery fence と公開 reason。

隔離 worktree 内の Code / Diff と Git 履歴自体は外部状態である。成果の統合は engine が無条件に実行せず、人間または親 Session が判断して通常の Git 操作として行う。定義上の `worktree` field は現時点では未解禁である。

## 使用禁止語

| 使用禁止語 | 正規語 | 理由 |
| --- | --- | --- |
| gate | completion | Node の完了定義と辺を混同する旧語であり、定義・schema・Diagnostic では使用しない |
| WorkflowRun / Run | WorkflowExecution | 定義の一回の実行は WorkflowExecution |
| StepExecution / WorkflowStep | NodeExecution | 実行単位は NodeExecution |
| ParallelRun / ParallelStep | Fanout / NodeExecution | 並列合成子は Fanout |
| ChildNodeDefinition | NodeDefinition | child も通常の NodeDefinition |
| NodeType | Node kind | kind は Session / Command / Fanout / Sequence |
| RunStatus | WorkflowExecution.status | 木全体 status の属性として扱う |
| ChatSession | Session または AgentSession | Node と provider identity のどちらかを明示する |
| ReviewThread / ReviewComment | Thread / Comment | review 固有に分けない |
| PtySession | Terminal / TerminalSurface | product 語彙は Terminal |
| WorkflowEvent | durable workflow fact | domain entity ではなく記録された事実 |

## Diagnostic

Diagnostic は WorkflowDefinition の parse、shape、resolve、typecheck、control-flow の検証結果であり、実行木や NodeExecution の lifecycle state ではない。
