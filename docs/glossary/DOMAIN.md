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
| Session | provider CLI と継続対話し、delegate の child を部分木として持てる Node | workflow |
| Command | 非対話 command を一度実行する葉 Node | workflow |
| Fanout | children を並列に束ねる合成 Node | workflow |
| Sequence | children を時系列に束ね、辺を所有する合成 Node | workflow |
| completion | Node の完了に対する要求の集合。`require: approval` で承認、Session の `delegate` で同一会話内の続行条件を要求する。併記は and で、要求を省略すると本来の完了条件を使う | NodeDefinition / NodeExecution |
| 実行木（execution tree） | 実際に開始した NodeExecution が作る再帰木 | Worktree |
| 辺（edge） | Node completion 後の進行先。Sequence の children エントリが所有する | Sequence |
| 述語（Predicate） | 辺と completion の判断に使う真偽値の論理式。原子は Artifact の required boolean field への参照で、`and` / `or` で合成する | workflow |
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

WorkflowExecution は木全体の `Running` / `Completed` / `Aborted` を所有する。NodeExecution は `Running` / `WaitingApproval` / `Succeeded` / `Aborted` と completion signal を所有する。プロセスの在否は状態と別に読み取り、終わっていない Node のプロセスが居ない場合は介入待ちとして分類する。workflow aggregate だけが transition を決める。

Session の delegate は、親 Session が所有する同一 session 継続機構である。Artifact の提出を起点に child を実行し、結果により親の続行・完了を決める。child の NodeExecution は親 Session の部分木であり、発火ごとに新しい NodeExecution と attempt を持つ。親の NodeExecution・attempt・AgentSession は維持する。child の結果、発火回数、注入済みの事実から続行状態を導出し、親 Artifact の `child` は engine が管理する。

親 Session の Resume に新しい attempt が必要な場合、未完了の委任ラウンドを新しい親 attempt が引き継ぐ。既存 child の親IDと worktree の継承先は履歴として保持し、child の完了・再試行は attempt 間の対応から最新の親の継続状態へ反映する。

### AgentSession

AgentSession は provider、provider session identity、opaque transcript reference、Terminal ownership、open / paused / archived lifecycle、および agent の活動状態を持つ。活動状態は `Working` / `AwaitingAnswer` / `AwaitingInstruction` の3値であり、lifecycle とは独立して AgentSession が所有する。NodeExecution / WorkflowExecution はこの活動状態を持たない。conversation 本文と provider UI は provider CLI / transcript が所有する。単独 Session の lifecycle も実行木から分離した別の作業モデルにはしない。

### 隔離 worktree

`isolated` を宣言した NodeExecution は attempt ごとに、親 worktree の HEAD から branch と worktree を生成し、そこを実行コンテキストにする。`shared`（省略時を含む）は親から継承する。実行木の所属は root の Worktree のままであり、隔離 worktree は Workspace にならない。

branch は `releash/isolated/<node_execution_id>-a<attempt>`、path は `<repository root の親>/<repository 名>-worktrees/.releash-isolated/<node_execution_id>-a<attempt>` である。命名に埋め込まれた NodeExecution と attempt、および実行木の状態だけで識別する。Worktree 管理の一覧には、実行中も終了後も再起動後も現れない。branch/path は Node 詳細と Artifact から観測できる。Thread は読み側で所有実行木の Workspaceへ結び付ける。

隔離 worktree 内の Code / Diff と Git 履歴は外部状態である。engine は成果の統合も worktree・branch の削除も行わない。統合は人間または親 Sessionが判断して通常の Git 操作として行う。逐次 Nodeで `isolated` を使った成果は隔離 branchに残り、親 worktree で動く後続 Nodeには見えない。Session の隔離 worktree の実体が失われた場合、Resume は新しい attempt の worktree と provider session を作り、最初の指示を送る。Command の Retry は実行 worktree のフォルダを必要とする。Abort はフォルダの存在を確かめずに実行を中止する。

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
