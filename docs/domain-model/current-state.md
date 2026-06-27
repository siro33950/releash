# Releash ドメインモデル Current State Snapshot

## 目的

現行コードに存在する語彙と、正規語との差分を記録する。
正規語・構造・状態所有・境界の定義は [../architecture/GLOSSARY.md](../architecture/GLOSSARY.md) を参照する。

このドキュメントは定義ではなく、現行実装スナップショットである。

## 対象

Issue [#1176](https://github.com/siro33950/releash/issues/1176) のスコープに合わせ、主に以下を対象にする。

- `src-tauri/src/domain/workflow`
- `src-tauri/src/domain/code`
- `src-tauri/src/domain/repository`
- `src-tauri/src/domain/agent_session`

ただし、語彙の揺れが強く関係するため、必要に応じて `workspace_state`、`pty_session`、review/comment DTO、config/integration 由来の語彙も含める。

## 分類

| 分類 | 意味 |
|---|---|
| canonical | 正規語として採用する |
| legacy_name | 現行実装名または旧語彙。正規語へ読み替える |
| internal | engine / service / validation などの内部語彙 |
| read_model | UI/API/一覧/同期/projection 用の表現 |
| external | 外部 repository / integration / hosting service 側の情報 |
| attribute | Entity ではなく属性として扱う |
| not_adopted | 正規語として採用しない |

## 現行コード上の主要語彙

### workflow

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `WorkflowDefinition` | WorkflowDefinition | canonical | workflow の定義。 |
| `NodeDefinition` | NodeDefinition | canonical | WorkflowDefinition 内の作業単位の定義。 |
| `WorkflowExecution` | WorkflowExecution | canonical | WorkflowDefinition の一回の実行。 |
| `WorkflowRun` | WorkflowExecution | legacy_name | `Run` は旧実装語彙。 |
| `WorkflowRunRecord` | WorkflowExecution | legacy_name | 永続化表現。正規語は WorkflowExecution。 |
| `WorkflowRunSummary` | WorkflowExecution | read_model | 一覧用 summary。Entity ではない。 |
| `RunId` | `WorkflowExecution.id` | attribute | id 属性として扱う。 |
| `RunStatus` | `WorkflowExecution.status` | attribute | status 属性として扱う。 |
| `TerminalRunStatus` | `WorkflowExecution.status` | internal | 終了状態だけを切り出した実装型。 |
| `TriggerSource` | `WorkflowExecution.created_from` | attribute | 起動元属性として扱う。 |
| `WorkflowName` | `WorkflowDefinition.name` | attribute | name 属性として扱う。 |
| `NodeName` | `NodeDefinition.name` | attribute | name 属性として扱う。 |
| `WorktreePath` | `Workspace.worktree_ref.path` | attribute | Worktree 参照の path。 |
| `NodeType` | なし | not_adopted | NodeDefinition の構成・参照先・実行内容から判断する。 |
| `ChildNodeDefinition` | NodeDefinition | legacy_name | fanout 先も通常の NodeDefinition として扱う。 |
| `WorkflowExecutionState` | `WorkflowExecution.status` | attribute | lifecycle status として扱う。 |
| `WorkflowSummary` | WorkflowDefinition | read_model | WorkflowDefinition の summary。 |
| `WorkflowStateSnapshot` | なし | read_model | snapshot 実装。 |
| `WorkflowEvent` | なし | not_adopted | 現時点では domain entity として採用しない。 |
| `WorkflowStepContext` | なし | internal | NodeExecution が Session / Command に渡す実行 context。 |
| `workflow_variables` | なし | internal | 実行時の変数展開用 data。 |

### workflow fanout / parallel

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `ParallelRunState` | Fanout | legacy_name | parallel 系語彙は Fanout に吸収する。 |
| `ParallelChildRun` | NodeExecution / Fanout | legacy_name | fanout child も NodeExecution。 |
| `ParallelChildState` | NodeExecution / Fanout | legacy_name | fanout child の状態。 |
| `ParallelAggregate` | Fanout | legacy_name | fanout 集約設定/処理として扱う。 |
| `ParallelStepState` | Fanout | read_model | UI/API 用 state。 |
| `NodeCompletion` | NodeExecution | internal | NodeExecution 完了時の処理入力。 |
| `ParallelChildCompletion` | NodeExecution | internal | child NodeExecution 完了時の処理入力。 |
| `ParallelReduceResult` | Fanout | internal | fanout 集約処理結果。 |
| `ParallelChildOutputMerge` | Fanout | internal | fanout child output merge 処理。 |
| `ParallelChildCompletionInput` | Fanout | internal | fanout child completion 入力。 |
| `ParallelParentTransitionPlan` | Fanout | internal | fanout parent 遷移処理計画。 |
| `ParallelParentCompletionPlan` | Fanout | internal | fanout parent completion 処理計画。 |
| `SubmissionParallelRun` | Fanout | internal | output submission 判定用表現。 |
| `SubmissionParallelChild` | Fanout | internal | output submission 判定用表現。 |
| `SubmissionParallelChildState` | Fanout | internal | output submission 判定用表現。 |

### workflow engine internal

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `NextNodeDecision` | なし | internal | engine の内部判断結果。 |
| `CycleGuardDecision` | なし | internal | engine の内部安全判定結果。 |
| `TurnCompleteDecision` | なし | internal | Turn 完了時の内部判断結果。 |
| `TurnCompleteMutationPlan` | なし | internal | engine の内部 mutation plan。 |
| `ApprovalTransitionDecision` | なし | internal | approval 遷移判断。 |
| `ApprovalApplication` | なし | internal | approval 適用処理表現。 |
| `ApprovalCompletion` | なし | internal | approval completion 表現。 |
| `ApprovalApplicationTransition` | なし | internal | approval 遷移表現。 |
| `ApprovalApplicationPlan` | なし | internal | approval 適用計画。 |
| `ApprovalInputError` | Diagnostic | internal | validation error。 |
| `ApprovalRuleError` | Diagnostic | internal | validation error。 |
| `ApprovalChatInstructionContext` | なし | internal | approval chat 処理用 context。 |
| `ApprovalChatSessionSnapshot` | なし | internal | approval chat 処理用 snapshot。 |
| `ApprovalTargetSnapshot` | なし | internal | approval 対象検証用 snapshot。 |
| `TransitionRule` | なし | internal | engine の transition 設定。 |
| `CycleGuard` | なし | internal | engine の安全設定。 |
| `CollectConfig` | なし | internal | engine の collect 設定。 |
| `ReduceStrategy` | なし | internal | engine の reduce 設定。 |

### workflow contract / output

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `FacetKind` / `FacetKey` / `FacetSummary` | Facet | canonical | NodeDefinition から参照される補助部品。 |
| `ResolvedFacets` | Facet | internal | Facet の解決状態。 |
| `ContractType` | Contract | canonical | output contract の型表現。 |
| `ContractValidationResult` | Contract | internal | validation result。 |
| `ContractViolation` | Contract / Diagnostic | internal | contract validation のエラー詳細。 |
| `ContractLookupError` | Diagnostic | internal | contract lookup error。 |
| `OutputSubmittedSnapshot` | Artifact | internal | output submit 処理用 snapshot。 |
| `ContractValidationMetadata` | Contract | internal | validation metadata。 |
| `ConditionalArrayRule` | Contract | internal | validation rule。 |
| `WorkflowValidateOutputResult` | Contract / Diagnostic | internal | output validation result。 |
| `SubmitOutputCommand` | Command | internal | usecase command input。 |
| `CollectedOutputEntry` | Artifact | internal | collect 処理の内部 entry。 |
| `StepOutput` | Artifact | legacy_name | NodeExecution output の現行表現。 |
| `StepHistoryEntry` | NodeExecution / Artifact | legacy_name | history/output の現行表現。 |
| `ChildOutputSnapshot` | Fanout / Artifact | legacy_name | fanout child output snapshot。 |
| `TokenUsage` | なし | internal | measurement。 |

### code

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `Hunk` | CodeAnchor / Diff | read_model | Diff 表示・参照用の範囲表現。 |
| `ChangeGroup` | Diff | read_model | Diff 表示用 grouping。 |
| `HiddenRange` | Diff | read_model | 表示用 range。 |
| `VisibleBlock` | Diff | read_model | 表示用 block。 |
| `DiffFileEntry` | Diff | read_model | diff tree 表示用 entry。 |
| `DiffTreeNode` | Diff | read_model | diff tree 表示用 node。 |
| `MentionReference` | CodeAnchor | legacy_name | code 位置参照。 |
| `ReviewBase` | Diff | read_model | review view の表示条件。 |
| `ReviewSection` | Diff | read_model | review view の表示条件。 |
| `ReviewLimitReason` | Diff | read_model | review view の表示制限理由。 |
| `ReviewBlobContentType` | Code | read_model | review view の表示判定。 |
| `ReviewThresholds` | Diff | read_model | review view の表示閾値。 |

### repository

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `Worktree` | Worktree | canonical | Repository の特定 checkout / working tree。 |
| `Branch` | Repository | external | repository 側の情報。 |
| `Commit` | Repository | external | repository 側の情報。 |
| `FileStatus` | Diff / Repository | read_model | repository status の表示用情報。 |
| `FileDiffStat` | Diff / Repository | read_model | diff stat。 |
| `RepositoryStatusScan` | Repository | read_model | status scan result。 |
| `WorktreePrEntry` | なし | external | external repository/hosting service 側の情報。 |
| `WorktreePrStatusSync` | なし | external | external repository/hosting service 側の sync message。 |
| `PrInfo` / `PrStatus` / `PrDetail` | なし | external | external repository/hosting service 側の情報。 |
| `PrReview` / `PrComment` / `PrAuthor` | なし | external | external repository/hosting service 側の情報。 |
| `IssueInfo` / `IssueLabel` / `Milestone` | なし | external | external issue tracker 側の情報。 |
| `AheadBehind` / `ProviderStatus` | なし | external | external repository/hosting service 側の情報。 |

### agent_session

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `ChatSession` | Session | legacy_name | 正規語は Session。 |
| `ChatMessage` | Message | legacy_name | 正規語は Message。 |
| `MessageRole` | MessageRole | canonical | Message の role。 |
| `MessagePart` | MessagePart | canonical | Message の部分表現。 |
| `PermissionRequest` | PermissionRequest | canonical | Turn に属する許可要求。 |
| `AttachmentRef` | Attachment | legacy_name | Attachment の参照表現。 |
| `SessionAttachment` | Attachment | legacy_name | Attachment に吸収。 |
| `ImageAttachment` | Attachment | legacy_name | Attachment に吸収。 |
| `ActivityEntry` | MessagePart | legacy_name | MessagePart の内部表現。 |
| `QueuedAgentTurn` | Turn | read_model | queued turn の表示/管理表現。 |
| `TurnPhase` | Turn | read_model | Turn の表示用 phase。 |
| `TurnEventLog` | Turn | read_model | Turn の event log 実装。 |
| `SessionState` | Session | read_model | 実装上の状態分類。 |
| `PlanMode` | Session | attribute | Session UI/agent 実行設定。 |
| `PermissionMode` | `Session.permission_mode` | attribute | Session の許可モード。 |
| `LegacyPermissionMode` | `Session.permission_mode` | legacy_name | legacy compatibility。 |
| `ContextCarryState` | Session | internal | Session 復元/再注入の内部状態。 |
| `AgentEditorContext` / `AgentEditorSelection` | Session | internal | agent input 用 editor context。 |
| `ModelInfo` / `ModelId` / `ModelEntry` | なし | external | agent backend/model metadata。 |
| `SkillEntry` / `SlashCommand` | なし | external | agent runtime/UI command metadata。 |

### workspace_state / terminal / review / integration

| 現行語彙 | 正規語 | 分類 | 理由 |
|---|---|---|---|
| `WorkspaceState` | WorkspaceState | canonical | Workspace の UI state。 |
| `WorkspaceTabsState` | WorkspaceState | read_model | WorkspaceState の内部表現。 |
| `WorkspaceLayoutState` | WorkspaceState | read_model | WorkspaceState の内部表現。 |
| `WorkspaceTabEntry` | WorkspaceState | read_model | WorkspaceState の内部表現。 |
| `PtySession` | Terminal | legacy_name | product/domain 語彙は Terminal。 |
| `PtySessionRegistry` | Terminal | internal | Terminal backend 管理実装。 |
| `PtyKind` / `PtyEvictReason` / `PtyLifecycleConfig` | Terminal | internal | Terminal backend 実装語彙。 |
| `ReviewThread` | Thread | legacy_name | Thread に吸収。 |
| `ReviewComment` | Comment | legacy_name | Comment に吸収。 |
| `NotificationEvent` / `NotifyConfig` / `DesktopNotifyMode` | なし | external | notification integration の event/config。 |
| `RemoteAccess` / `DetectedInterface` / `VpnInterface` / `QrCodeResult` | なし | external | remote access integration。 |
| `HooksStatus` | なし | external | hooks integration。 |
| `ExternalEditor` / `EditorInfo` | なし | external | external editor integration。 |
| `NotionRepoConfig` / `NotionPropertyMapping` | なし | external | Notion integration config。 |

## 正規語に未対応の主要概念

| 正規語 | 現行状態 |
|---|---|
| NodeExecution | 未実装。現行は WorkflowExecution 内の step state / history として表現されている。 |
| Fanout | 未実装。現行は parallel 系語彙で表現されている。 |
| Task | 未実装。 |
| Artifact | 未実装。現行は StepOutput / StepHistoryEntry / ChildOutputSnapshot などで部分的に表現されている。 |
| Workspace | 独立 entity としては未実装。現行は `workspace_id ≒ worktree_path`。 |
| Command | 未実装。 |
| Thread / Comment | 正規語としては未実装。現行は review DTO 名で表現されている。 |

## 未合意語彙

現時点ではなし。
