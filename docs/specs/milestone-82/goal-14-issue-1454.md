# Goal: #1454 Workspace UIをNode中心の再帰ツリーに統一

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1454 --repo siro33950/releash` でissue本文を読むこと。

本goalはGoal 1〜13完了後に追加されたWorkspace UIのfollow-upである。完了済みissueのdomain/event仕様を再実装しない。既存のWorkflowExecution / NodeExecution / Fanoutを入力として、Workspace UI専用のRust-owned projectionと表示境界を置き換える。

## UIモデル

- `NewSession`が作るものはWorkflowに属さない単独Node。contentはSession。
- Workflowは複数Nodeを束ねるbranch。
- FanoutはWorkflow内で複数Nodeを束ねるbranch。
- Nodeは`Session | Command` contentを持つleaf。
- Workflow/Fanoutは独自の中央viewを持たない。
- Workspaceは一つの再帰tree、中央は一つの`NodeContentView`とする。

## 実装内容

1. **再帰Workspace read model**: `WorkspaceTreeNodeDto::Session | Workflow { node_executions }`を`Node | Workflow | Fanout`の再帰enumへ置き換える。Nodeにchildrenを持たせず、Workflow/Fanoutにcontentを持たせない。tree summaryはtitle/status/content kind/capability/updated timeだけを返し、Session本文・Command output・Artifact本文を含めない。
2. **Rust projection**: 非Workflow Sessionを単独Nodeへ、通常Session/Command定義をNodeへ、fanout親をFanout branchへ投影する。WorkflowDefinitionの宣言順を基準にし、fanout childをWorkflow直下へ重複表示しない。未開始Nodeはqueued。retry/loopのattemptは同じUI Nodeへ集約する。status/capability/title/order/fanout階層化をfrontendへ置かない。
3. **汎用Node detail**: `get_workspace_workflow_node_detail`を`get_workspace_node_detail(worktree_path, node_id)`へ置き換える。contentは`Session { session_id? } | Command { display_command?, result? }`。Node IDはfrontendが解析しないopaque IDとし、retry後も同じUI Node IDを維持する。
4. **Session表示の統一**: 単独SessionとWorkflow Sessionの両方を同じ`BoundSessionChat`で表示する。`workflowNodeSession`による中央表示除外を削除し、会話本文、入力、live更新、permission応答を利用可能にする。
5. **中央表示の一本化**: `MainLayout`の`AgentChatPanel | WorkflowView`分岐と`WorkflowView`を削除し、常に`ViewToolbar + NodeContentView`を表示する。Workflow/Fanout行はtreeの展開・折り畳みだけを行い、中央表示を変更しない。
6. **Command snapshot**: Artifact参照展開後のCommandをsecret maskして`CommandPrepared` eventへ保存し、event projectionでNodeExecutionの`display_command`へ復元する。raw Commandは永続化しない。event commit成功後にprocessをspawnする。Command detailはdisplay command/status/exit code/duration/stdout/stderrを表示する。
7. **選択と操作**: `CenterSelection`を`{ kind: "node", worktreePath, nodeId }`へ統一する。NewSessionは選択variantではなく作成操作とする。初回はbackendの`preferred_node_id`を使うが、更新時に表示中Nodeを勝手に変更しない。Workflow actionとNode actionはbackend capabilityだけに従う。
8. **内部metadataを非表示化**: execution ID、NodeExecution ID、attempt、fanout parent attempt、item/child index、raw Node kind、resume checkpointをtree row/Node headerへ表示しない。

## 削除対象

- `WorkflowView`と専用detail hook/type
- `agentSession | workflowNode`の中央表示分岐
- flat `nodeExecutions` Workspace DTO
- frontendでのfanout階層化、attempt/status/capability集約
- `#attempt`、`item N`、`child N`、内部IDを表示するUI
- Workflow Sessionを通常Session表示から除外する条件

## テスト

- Rust projection: 単独Session、通常Workflow、queued Node、Fanout、item展開、retry、missing Session、status/capability集約。
- event replay: `CommandPrepared`からmasked実行Commandを復元し、raw secretを保存・表示しない。
- frontend: recursive tree、branch toggle、Node選択、単独/Workflow共通Session表示、Command表示、内部metadata非表示。
- integration: NewSession、Workflow Session、Workflow Command、Fanout、retry後の選択維持。

## 受け入れ基準

- NewSession NodeとWorkflow Session Nodeの両方で完全なSession本文と入力欄が表示される。
- Workflow Sessionのlive更新を監視し、入力とpermission応答ができる。
- Command Nodeでその実行のmasked Commandと結果を確認できる。
- Workflow/Fanoutはtree branchとしてだけ存在し、独自中央viewを持たない。
- Fanout childがWorkflow直下へflat表示されず、retryごとにNode行が増えない。
- attempt、item/child座標、内部UUIDがUIへ表示されない。
- tree summaryにSession本文、Command output、Artifact本文を含めない。
- Tauri/WebSocketから同じWorkspace tree/detail read modelを再利用できる。

## 対象外

- WorkflowExecution / NodeExecution domain modelの廃止。
- CLI / Local APIのNodeExecution識別規則変更。
- Command stdout/stderrのstreaming対応。
- 過去attemptをtimelineとして常時表示するUI。
