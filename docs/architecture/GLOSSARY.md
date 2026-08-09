# Releash GLOSSARY

## 目的

Releash のドメイン横断ユビキタス言語を定義する。
このドキュメントは [#1176](https://github.com/siro33950/releash/issues/1176) の正規成果物であり、設計、レビュー、移行作業で参照する canonical vocabulary として扱う。

このドキュメントは現行実装の棚卸しではなく、Releash で使う正しい語彙と構造を定義する。

## 正規語一覧

| 正規語 | 定義 | 所属ドメイン | 使用禁止語 | 備考 |
|---|---|---|---|---|
| WorkflowDefinition | workflow の定義。NodeDefinition の構成、順序、条件、入力を持つ。 | workflow | Workflow, WorkflowBundle | 実行状態は持たない。 |
| NodeDefinition | WorkflowDefinition 内の作業単位の定義。 | workflow | StepDefinition, ChildNodeDefinition, NodeType | 何を扱うかは構成・参照先・実行内容から決まる。 |
| WorkflowExecution | WorkflowDefinition の一回の実行。 | workflow | WorkflowRun, Run | `status` を持つ。 |
| NodeExecution | NodeDefinition の一回の実行。 | workflow | StepExecution, WorkflowStep | Session / Command / Fanout を扱うことがある。 |
| Fanout | 親 NodeExecution から展開された子 NodeExecution 群を束ねる実体。 | workflow | ParallelRun, ParallelChildRun, ParallelStep | `parallel` 系語彙は Fanout に吸収する。 |
| Artifact | WorkflowExecution / NodeExecution の間で生成・参照される Object。 | workflow | StepOutput, ArtifactSnapshot, ArtifactLineage | 状態は持たない。意味と schema はユーザーまたは WorkflowDefinition が決める。 |
| Facet | NodeDefinition から参照される再利用可能な補助部品。 | workflow | ResolvedFacets | 主要 Entity ではない。 |
| Contract | NodeDefinition の output contract や Artifact/output validation に関わる補助語彙。 | workflow | ContractValidationMetadata | 主要 Entity ではない。 |
| Diagnostic | 定義や参照の構文・validation error。 | workflow | lifecycle state | lifecycle state ではない。 |
| Workspace | Releash 側の作業コンテキスト。Worktree を参照するが同一ではない。 | workspace_state | Worktree | Releash 所有状態の帰属先。 |
| WorkspaceState | Workspace の UI state。 | workspace_state | WorkspaceTabsState, WorkspaceLayoutState, WorkspaceTabEntry | editor tabs / layout / selected diff file など。 |
| Worktree | Repository の特定 checkout / working tree。 | repository | Workspace | 外部実体。Releash は所有しない。 |
| Repository | Worktree の背後にある履歴・remote・branch のまとまり。 | repository | Repo | Releash は所有しない。 |
| Code | Worktree 内のファイル内容。 | code | FileContent | Releash は所有せず、参照・表示・操作する。 |
| Diff | Worktree / Repository の状態から計算される差分。 | code | CodeDiff | Releash が生み出すものではない。固定した判断材料は Artifact にできる。 |
| CodeAnchor | Code 上の位置への参照。 | code | HunkRef, MentionReference | 命名は仮。Code 本体ではない。 |
| Session | Agent との継続的な対話・実行単位。 | agent_session | ChatSession | Workspace に属する。 |
| Turn | Session 内の一回の agent interaction / execution。 | agent_session | AgentTurn | PermissionRequest は Turn に属する。 |
| Message | Session / Turn 内の message。 | agent_session | ChatMessage | MessageRole / MessagePart を持つ。 |
| MessagePart | Message の部分表現。 | agent_session | ActivityEntry | variant は独立語彙にしない。 |
| MessageRole | Message の役割。 | agent_session | Role | human / agent / system。 |
| PermissionRequest | Turn に属する個別の許可要求。 | agent_session | ApprovalDecision | PermissionDecision は解決結果の属性として扱う。 |
| Attachment | Message に添付される入力・参照。 | agent_session | AttachmentRef, SessionAttachment, ImageAttachment | Artifact とは別。 |
| Thread | Workspace に属する会話・作業履歴・文脈。 | comment | ReviewThread | WorkflowExecution / NodeExecution には属さない。 |
| Comment | Thread 配下の comment。 | comment | ReviewComment | review comment / discussion comment を吸収する。 |
| Command | Session と対比される non-interactive な一回の command 実行単位。 | workflow | CommandExecution, ShellCommand, RunCommand | 命名は暫定。 |
| Terminal | ユーザーが操作する interactive shell session。 | terminal_surface | PtySession, Command | Workspace に属する。workflow / node が直接触るものではない。 |
| TerminalSurface | 正規語 Terminal の backend 実装語彙（terminal_surface ドメインの集約）。 | terminal_surface | PtySession | product/domain 語彙は Terminal。 |
| TerminalSurfaceOwner | TerminalSurface の所有者（Workspace または Session）。 | terminal_surface | - | 正規語 Terminal の backend 実装語彙。 |
| UI | 人間が画面から操作・観測する面。 | operation_surface | FrontendDomain | Operation Surface。 |
| CLI | command line から操作・観測する面。 | operation_surface | CliMutationRequestRecord | Operation Surface。 |
| API | 外部 system、automation、remote client が programmatic に操作・観測する面。 | operation_surface | ProtocolDomain | Operation Surface。 |

## 構造

### Global / App

```text
Global / App
  └─ WorkflowDefinition
       └─ NodeDefinition
```

- WorkflowDefinition は Global / App に属する。
- NodeDefinition は WorkflowDefinition に属する。
- NodeDefinition は standalone では存在しない。

### Workspace

```text
Workspace
  ├─ targets Worktree
  ├─ Terminal
  ├─ WorkflowExecution
  │    ├─ Artifact
  │    └─ NodeExecution
  │         └─ Fanout
  │              ├─ parent NodeExecution
  │              └─ child NodeExecution[]
  │
  ├─ Session
  │    ├─ Turn
  │    │    └─ PermissionRequest
  │    └─ Message
  │         └─ Attachment
  ├─ Command
  ├─ Thread
  │    └─ Comment
  └─ WorkspaceState
```

- Workspace は Releash 側の作業コンテキスト。
- Workspace は Worktree を参照するが、Worktree そのものではない。
- WorkflowExecution / Session / Command / Terminal / Thread / WorkspaceState は Workspace に属する。
- Artifact / NodeExecution は WorkflowExecution に属する。
- Fanout は親 NodeExecution と子 NodeExecution 群を束ねる。
- PermissionRequest は Turn に属する。
- Thread は CodeAnchor を参照できるが、CodeAnchor を所有しない。
- Thread は WorkflowExecution / NodeExecution には属さない。
- Task は Releash core Entity として持たない。task 的な一覧は Artifact のユーザー定義 field（例: `plan.tasks`）として表現できる。

### Worktree / Repository / Code

```text
Worktree
  ├─ Code
  │    └─ CodeAnchor
  └─ Diff

Repository
  └─ Worktree
```

- Worktree は Repository の特定 checkout / working tree。
- Code は Worktree 内のファイル内容。
- CodeAnchor は Code 上の位置への参照。
- Diff は Worktree / Repository の状態から計算される。
- Worktree / Repository / Code / Diff は Releash が所有する状態ではない。
- CodeAnchor は Code 本体ではなく、Thread などから参照される位置情報。

### Operation Surface

```text
Operation Surface
  ├─ UI
  ├─ CLI
  └─ API
```

- UI / CLI / API は domain entity ではなく操作・観測の入口。
- Operation Surface は domain state を所有しない。

## 状態所有

### 状態を持つ

- Workspace
- WorkflowExecution
- NodeExecution
- Fanout
- Session
- Turn
- PermissionRequest
- Command
- Terminal
- Thread
- WorkspaceState
- WorkflowDefinition（管理状態）

### 状態を持たない

- Artifact
- Worktree（Releash 側の状態は持たない）
- Repository（Releash 側の状態は持たない）
- Code（Releash 側の状態は持たない）
- Diff（Releash 側の状態は持たない）
- NodeDefinition

### Diagnostic

- WorkflowDefinition の構文・validation error
- NodeDefinition の構文・参照・遷移 error

構文エラー、validation error、参照エラーは lifecycle state ではなく Diagnostic として扱う。

## 外部実体との境界

- Workspace は Worktree を参照するが、Worktree と同一ではない。
- Worktree / Repository / Code / Diff は外部 repository 側の実体または派生 view。
- Releash が固定した判断材料として保持したい場合は Artifact にする。
- PR / Issue / Notion / external editor / remote access / notification は、それぞれ外部 system または integration の情報であり、現時点では core domain entity として扱わない。

## 使用禁止語一覧

| 使用禁止語 | 正規語 | 理由 |
|---|---|---|
| WorkflowRun | WorkflowExecution | `Run` は旧実装語彙。定義の一回の実行は WorkflowExecution と呼ぶ。 |
| Run | WorkflowExecution | 同上。 |
| RunId | WorkflowExecution.id | id 属性として扱う。 |
| RunStatus | WorkflowExecution.status | status 属性として扱う。 |
| TerminalRunStatus | WorkflowExecution.status | 終了状態だけを切り出した実装型。 |
| TriggerSource | WorkflowExecution.created_from | 起動元属性として扱う。 |
| WorkflowName | WorkflowDefinition.name | name 属性として扱う。 |
| NodeName | NodeDefinition.name | name 属性として扱う。 |
| WorktreePath | Workspace.worktree_ref.path | Worktree 参照の path 属性として扱う。 |
| NodeType | なし | NodeDefinition の構成・参照先・実行内容から判断する。 |
| ChildNodeDefinition | NodeDefinition | fanout 先も通常の NodeDefinition として扱う。 |
| StepExecution | NodeExecution | step ではなく node execution と呼ぶ。 |
| WorkflowStep | NodeExecution / NodeDefinition | 文脈に応じて定義か実行に分ける。 |
| ParallelRun | Fanout | parallel 系語彙は Fanout に吸収する。 |
| ParallelChildRun | Fanout / NodeExecution | fanout child も NodeExecution。 |
| ParallelStep | Fanout / NodeExecution | parallel ではなく fanout として扱う。 |
| ChatSession | Session | session の UI/DTO 名として扱い、正規語は Session。 |
| ChatMessage | Message | message の UI/DTO 名として扱い、正規語は Message。 |
| ActivityEntry | MessagePart | MessagePart の内部表現。 |
| AttachmentRef | Attachment | 参照表現であり、正規語は Attachment。 |
| SessionAttachment | Attachment | 同上。 |
| ImageAttachment | Attachment | 同上。 |
| ReviewThread | Thread | review 固有に分けず Thread に吸収する。 |
| ReviewComment | Comment | review 固有に分けず Comment に吸収する。 |
| PtySession | Terminal / TerminalSurface | 旧 backend 実装語彙（pty_session ドメインは terminal_surface へ改名済み）。product/domain 語彙は Terminal、backend 実装語彙は TerminalSurface。 |
| WorkflowEvent | なし | 現時点では domain entity として採用しない。 |
| ActionPlan | なし | 合意済み語彙ではない。 |
| Notification | なし | 今は Entity にしない。 |
