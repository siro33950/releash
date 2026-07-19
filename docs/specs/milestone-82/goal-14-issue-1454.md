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
- 旧Agent command paletteと設定可能なAgent shortcutは廃止する。Session内のFind/Raw scrollback/Copy toolbarと固定`Cmd/Ctrl+F`・`Ctrl+O`は維持する。

## 実装内容

1. **再帰Workspace read model**: `WorkspaceTreeNodeDto::Session | Workflow { node_executions }`を`Node | Workflow | Fanout`の再帰enumへ置き換える。Nodeにchildrenを持たせず、Workflow/Fanoutにcontentを持たせない。tree summaryはtitle/status/content kind/capability/updated timeだけを返し、Session本文・Command output・Artifact本文を含めない。
2. **Rust projection**: 非Workflow Sessionを単独Nodeへ、通常Session/Commandの各実行occurrenceをNodeへ、fanout親の各実行occurrenceをFanout branchへ投影する。Workspace treeにはdurableな`NodeStarted`に由来する実在の`NodeExecution`だけを表示し、未開始の定義Nodeやfanout expected slotから行を合成しない。実行開始済みNodeはevent projectionが復元した実行順を保ち、同じ定義Nodeが反復された場合も`A → B → A → C`のように実行ごとに別行を作る。fanout childは対応するFanout branch配下へ実行順に投影し、Workflow直下へ重複表示しない。この表示契約は[`specs/unified-node-model/decisions.md` §実行木](../../../specs/unified-node-model/decisions.md#実行木)の「実行木には展開結果（実際に起きたこと）だけが載る」という決定に従う。status/capability/title/order/fanout階層化をfrontendへ置かない。
3. **汎用Node detail**: `get_workspace_workflow_node_detail`を`get_workspace_node_detail(worktree_path, node_id)`へ置き換える。contentは`Session { session_id? } | Command { display_command?, result? }`。実行occurrenceごとに異なるNode IDを返し、frontendが解析しないopaque IDとする。各IDは後続occurrenceが追加されても変えず、detailはそのIDが指す実行occurrenceのcontentを返す。
4. **Session表示の統一**: 単独SessionとWorkflow Sessionの両方を同じ`BoundSessionChat`で表示する。`workflowNodeSession`による中央表示除外を削除し、会話本文、入力、live更新、permission応答を利用可能にする。
5. **中央表示の一本化**: `MainLayout`の`AgentChatPanel | WorkflowView`分岐と`WorkflowView`を削除し、常に`ViewToolbar + NodeContentView`を表示する。Workflow/Fanout行はtreeの展開・折り畳みだけを行い、中央表示を変更しない。
6. **Command snapshot**: Artifact参照展開後のCommandをsecret maskして`CommandPrepared` eventへ保存し、event projectionでNodeExecutionの`display_command`へ復元する。raw Commandは永続化しない。event commit成功後にprocessをspawnする。Command detailはdisplay command/status/exit code/duration/stdout/stderrを表示する。
7. **選択と操作**: `CenterSelection`を`{ kind: "node", worktreePath, nodeId }`へ統一する。NewSessionは選択variantではなく作成操作とする。初回はbackendの`preferred_node_id`を使い、更新時は選択IDが新snapshotの表示対象に残る間だけ表示中Nodeを維持する。Workflow actionとNode actionはbackend capabilityだけに従う。
8. **内部metadataを非表示化**: execution ID、NodeExecution ID、attempt、fanout parent attempt、item/child index、raw Node kind、resume checkpointをtree row/Node headerへ表示しない。
9. **旧Agent command hostの廃止**: Session tabに依存していたcommand palette、New/Search/Previous/Next thread command、Agent shortcut設定と公開Tauri commandを削除する。Session localなFind、Raw scrollback、Copyは`ChatSessionView`のtoolbarと固定shortcutで提供する。
10. **選択Nodeの消滅**: AppはWorktreeごとに`awaiting_initial | selected`のUI lifecycleを持つ。選択IDが新snapshotの表示対象に残る間は選択を維持し、表示対象から消えた場合は`awaiting_initial`へ戻って同じsnapshotの`preferred_node_id`を初回表示と同じ経路で適用する。`preferred_node_id`が`null`の間は未選択のまま初回選択資格を維持する。単独SessionのCloseは`close_workspace_node(worktree_path, node_id)`でopaque IDの解決をRustへ委ねる。
11. **NewSessionの冪等作成**: AppがWorktreeごとの作成requestを所有し、Worktree切り替えをcancelとして扱わない。Rustの`create_workspace_session`はrequest UUIDを永続Session IDとしてcheck-and-saveを直列化し、同一requestの再送・並行実行・再起動後retryで同じSessionを返す。作成commit後は保存済みSessionをcanonical resultとし、retry時のpermission/backend/model変更に左右されない。異なるWorktreeまたは単独Session以外とのID衝突は拒否する。frontendは同一attemptのtaskをMainLayoutで共有し、失敗時だけ同じrequest UUIDの新attemptとして明示retryする。

## 削除対象

- `WorkflowView`と専用detail hook/type
- `agentSession | workflowNode`の中央表示分岐
- flat `nodeExecutions` Workspace DTO
- frontendでの実行順決定、fanout階層化、status/capability判定
- `#attempt`、`item N`、`child N`、内部IDを表示するUI
- Workflow Sessionを通常Session表示から除外する条件
- Agent command palette、設定可能なAgent shortcut、発火元のない`agent-*` custom event

## テスト

- Rust projection: 単独Session、通常Workflow、未開始Node非表示と空のWorkflow branch、実行開始済みFanoutのactual child限定、item展開、retry/loopの実行occurrence順（`A → B → A → C`）、missing Session、occurrenceごとのstatus/capability。
- event replay: `CommandPrepared`からmasked実行Commandを復元し、raw secretを保存・表示しない。
- frontend: recursive tree、branch toggle、Node選択、単独/Workflow共通Session表示、Command表示、内部metadata非表示。
- frontend: Settingsに旧Agent shortcut設定を表示せず、Session local toolbarと固定shortcutは利用できる。
- frontend: detailのauthoritativeな`None`でstale中央表示を破棄し、取得errorでは現在detailを維持する。新snapshotの表示対象に選択IDが残る場合は現在選択を維持し、消えた場合は同じsnapshotの`preferred_node_id`へfallbackする。`preferred_node_id`が`null`なら未選択のまま初回選択資格を維持する。
- Rust: opaque Node Close、同一NewSession requestの逐次・並行・再起動後retry、payload不一致拒否、保存失敗後retry。
- integration: NewSession、選択中単独SessionのClose後のsnapshot membershipに基づくfallbackまたは未選択、空Worktreeの最初のWorkflow Node自動選択、Workflow Session、Workflow Command、Fanout、retry/loopで追加された各occurrence、snapshotに残る過去occurrenceの選択維持。

## 受け入れ基準

- NewSession NodeとWorkflow Session Nodeの両方で完全なSession本文と入力欄が表示される。
- Workflow Sessionのlive更新を監視し、入力とpermission応答ができる。
- Command Nodeでその実行のmasked Commandと結果を確認できる。
- Workflow/Fanoutはtree branchとしてだけ存在し、独自中央viewを持たない。
- 実行済みNodeが常に実行順に並び、同じ定義Nodeのretry/loopも実行occurrenceごとに別行として増える。
- Fanout childがWorkflow直下へflat表示されず、対応するFanout branch配下へ実行順に表示される。
- attempt、item/child座標、内部UUIDがUIへ表示されない。
- tree summaryにSession本文、Command output、Artifact本文を含めない。
- Tauri/WebSocketから同じWorkspace tree/detail read modelを再利用できる。
- 旧Agent command palette/shortcut commandは公開せず、Session localなFind/Raw scrollback/Copyは維持される。
- 選択中NodeのIDが新snapshotの表示対象から消えると、同じsnapshotの`preferred_node_id`へfallbackし、`preferred_node_id`が`null`なら未選択になる。
- 空Worktreeでは初回選択資格を維持し、最初のWorkflow Nodeが追加されると自動選択される。refresh後も選択IDがsnapshotに残る間は現在選択を維持し、retry/loopで追加された別occurrenceへ勝手に移動しない。
- NewSession作成中にWorktreeを切り替えて戻っても同じrequestからSessionを重複作成せず、非表示Worktreeで完了した結果が現在表示中の別Worktreeを奪わない。

## 対象外

- WorkflowExecution / NodeExecution domain modelの廃止。
- CLI / Local APIのNodeExecution識別規則変更。
- Command stdout/stderrのstreaming対応。
- Workspace treeとは別に過去実行をtimeline化する専用UI。
