# Design

Primary Spec: [requirements.md](requirements.md) / [behavior.md](behavior.md)

## The actual design

### Architecture

Workspaceの状態分類はRustの`domain/workspace_tree`が所有する。分類の入力は`WorkspaceTreeNode`が既に持つ詳細状態、`completion_signals`、`recovery_owner_reason`であり、frontendやDTO変換で再判定しない。この責務配置は[DOMAIN.md](../../architecture/DOMAIN.md)の「規則はdomainが所有する」と、[USECASE.md](../../architecture/USECASE.md)および[GATEWAY.md](../../architecture/GATEWAY.md)のread model規約に従う。

`WorkspaceTreeNode`の詳細状態と4分類を分離する。詳細状態はNodeイベント由来の状態として維持し、Sequence / Fanoutの子集約で上書きしない。4分類は詳細状態などから導出する別の値であり、Sequence / Fanoutだけが自分自身の分類と配下の分類を集約する。これにより、詳細状態を操作可否や詳細表示の正本として維持したまま、親行の表示分類をSequence / Fanoutで共通化する。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/domain/workspace_tree/value_objects/mod.rs` | 詳細状態から`Interrupted`を除き、4分類を表す`WorkspaceNodeStatusClassification`を追加する |
| `src-tauri/src/domain/workspace_tree/entities/mod.rs` | leafの分類規則とSequence / Fanoutの重大度集約を所有し、詳細状態と親の分類集約を分離する |
| `src-tauri/src/domain/workspace_tree/projection.rs` | runtime由来のcompletion signalとpaused状態を反映した後に分類を再計算する |
| `src-tauri/src/usecase/workflow/workspace_tree.rs` | ツリーDTOの`status`を4分類へ変更し、詳細DTOへ`statusClassification`を追加する |
| `src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs` | domainが確定した分類をツリーDTOと詳細DTOへ写像する |
| `src/types/workspace-tree.ts` | ツリー用4分類型と詳細状態型を分離する |
| `src/components/workspace/WorkflowNodeStatusIcon.tsx`、`WorkspaceBranchStatusIcon.tsx`、`WorkspaceList.tsx` | 色とpulseを4分類だけから選ぶ |
| `src/components/panels/NodeContentView/NodeContentView.tsx` | 詳細状態で形状を、`statusClassification`で色を選ぶ |

Workflow rootは引き続き`project_tree`で子へ平坦化し、表示対象にしない。現在の「最後の子からWorkflow rootの詳細状態を導出する」表示用分岐は廃止し、Workflow rootの実行状態とcapabilityは既存のexecution summaryおよびrecovery計算から維持する。

approve / retry / stop / resume / abort / archiveの可否、`resume_unavailable_reason`、`preferred_node_id`は4分類を入力にしない。これらは既存の詳細状態、completion signal、recovery情報、execution summaryから算出し続ける。workflowドメインの`ExecutionStatus`とそのTauri / API / CLI表現は変更しない。test専用の`ExecutionStatus::Interrupted`からWorkspaceの詳細状態を生成・公開する経路だけを除き、`WorkspaceNodeStatus::Interrupted`へ操作可否を依存させない。

#### 検証境界

- domainでは、leafの分類優先順位、`paused`とStop信号の組み合わせ、recovery fenceの優先、Sequence / Fanoutの同一重大度集約を検証する。
- gatewayでは、ツリーDTOが4分類だけを返すこと、詳細DTOが詳細状態と同じdomain分類を返すこと、capabilityとresume不能理由が変わらないことを検証する。
- frontendでは、4分類の色、ツリー行のpulse、詳細状態ごとのアイコン形状、ツリー型と詳細型の分離を検証する。

### Interface

公開command名、引数、失敗形式は変更しない。変更対象となるTauri read contractは次の三つである。

- `list_workspace_worktree_nodes`
- `get_workspace_tree_selection_reconciliation`
- `get_workspace_node_detail`

ツリー内の`WorkspaceNodeDto`、`WorkspaceSequenceDto`、`WorkspaceFanoutDto`は既存の`status` fieldを維持し、値だけを次のclosed setへ置き換える。

```text
status = "active" | "attention" | "failure" | "idle"
```

4分類のpublic stringは詳細状態のどの値とも重ならない語彙にする。同じtokenが分類と詳細状態の二つの意味を持つと、`idle`が詳細状態の完了として読まれ、`paused`と`aborted`のノードが完了として扱われる。

`WorkspaceNodeDetailDto`は詳細状態の`status`を維持し、4分類を`statusClassification`として追加する。

```text
status = "running" | "paused" | "failed" | "waiting" | "aborted" | "completed"
statusClassification = "active" | "attention" | "failure" | "idle"
```

`interrupted`はツリーの`status`、詳細の`status`、詳細の`statusClassification`のいずれにも現れない。分類は全到達可能な詳細状態に対してtotalであり、新しい失敗分類やfallback値を公開しない。

ツリーの`status`値変更はrendererと同時に切り替える互換性のないTauri response変更である。詳細の`statusClassification`は追加fieldだが、詳細`status`から`interrupted`を除く。local APIとCLIにはこのWorkspaceツリーread contractがないため、そのprotocolと応答は変更しない。

### Data Model

`WorkspaceNodeStatus`はNodeイベント由来の詳細状態を所有し、`Running | Paused | Failed | Waiting | Aborted | Completed`に閉じる。新しい`WorkspaceNodeStatusClassification`は`Active | Attention | Failure | Idle`に閉じ、public stringはそれぞれ`active | attention | failure | idle`とする。分類はvariant名でもpublic stringでも詳細状態と重ならず、型と語彙の双方で区別する。

`WorkspaceTreeNode`は詳細状態を正本として保持し、分類はそこにcompletion signalとrecovery情報を加えて導出した一時的なread stateとして保持する。Sequence / Fanoutの分類だけは、自分自身の分類と子の分類から再計算した値を保持する。分類にidentity、version、永続recordは追加しない。

ツリーDTOは分類だけを持ち、詳細状態、completion signal、recovery理由を重複して載せない。詳細DTOは詳細状態と分類の両方を持つ。既存の`submitReceived`、`stopReceived`、`waitingFor`、`recoveryReason`は詳細情報として維持する。

### Database

該当なし。分類は既存のcanonical event foldから復元した`WorkspaceTreeNode`上で導出し、SQLite schema、event、projection record、access pathを追加・変更しない。

### UI/UX

ツリー行は既存のBot / Terminal / ListTree / GitForkの形状を維持し、`status`の4分類だけで色を選ぶ。`active`は青、`attention`は黄、`failure`は赤、`idle`は緑に対応する。ツリー行の`title`と`aria-label`は現在と同じく行の`status`をそのまま含め、4分類だけを示す。詳細状態を行のテキストへ出さない。

詳細ペインの`WorkflowNodeStatusIcon`は、`status`でLoader / Clock / AlertTriangle / Circle / Ban / CheckCircleの既存の形状差を維持し、`statusClassification`で色を選ぶ。これにより、たとえばrecovery fenceを持つ`paused`はpausedの形状のまま赤になり、通常の`paused`は同じ形状のまま緑になる。frontendは二つのfieldから分類を再導出しない。

ツリー行のpulseは共有helperで4分類だけから決め、`active`と`attention`で有効、`failure`と`idle`で無効にする。詳細状態による形状選択とは独立させる。詳細ペインのアイコンはpulseさせず、`running`のspinを含む既存の形状表現を維持する。

### Algorithm

各ノードについて、まず自分自身の分類をR-002の順序どおり一つのordered matchで導出する。

1. `recovery_owner_reason`がある場合は`Failure`。
2. 詳細状態が`Failed`の場合は`Failure`。
3. 詳細状態が`Waiting`の場合は`Attention`。
4. 詳細状態が`Running`で、completion signalが`StopReceived`の場合は`Attention`。
5. 詳細状態が`Running`の場合は`Active`。
6. 詳細状態が`Paused | Completed | Aborted`の場合は`Idle`。

その後、実行木をpost-orderで一度走査し、Sequence / Fanoutごとに自分自身と直接の子が持つ確定済み分類から`Failure > Attention > Active > Idle`の最上位を採る。子の分類には既にその配下が畳み込まれているため、親ごとに全subtreeを再走査しない。internal rule recordとretry historyは現在の親子集約と同様に通常の配下から除外し、過去attemptは各leaf自身の分類で表示する。

`WorkspaceTree::restore`と`WorkspaceTreeProjector`の再計算に加え、`runtime_snapshot_nodes`がruntimeのcompletion signal、artifact、retry可否、paused状態を上書きした後にも同じ分類・親集約を実行する。これにより、ツリー全体のqueryと単独node detailのqueryが同じ最終入力から分類される。

### Infra

該当なし。

## Alternatives Considered

- frontendの色マップだけを差し替える案: recovery fenceとcompletion signalがツリーDTOに存在せず、分類規則もfrontendへ移るため不採用。
- ツリーDTOへ詳細状態を残したまま4分類fieldを追加する案: ツリーが使わない詳細状態と表示用分類が並び、呼び出し側がどちらを正とするか選べるため不採用。ツリーの`status`は4分類へ置き換え、詳細状態は詳細DTOだけに残す。
- SequenceとFanoutの既存集約を個別に修正する案: 同じ親行で異なる重大度規則が残るため不採用。domainの一つのpost-order集約を共有する。

## Cross-cutting concerns

- Performance: 分類と親集約は復元済みツリーに対する1回のpost-order処理とし、追加I/O、履歴全体の再読込、frontendでの再計算を増やさない。
- Compatibility: Tauriのツリーresponseはbackendとrendererを同時更新する。Workspaceツリーを持たないlocal API / CLI、workflow execution status、操作commandのcontractは変更しない。
- State ownership: 詳細状態、completion signal、recovery情報をsource of truthとし、4分類はdomainが導出するread stateに限定する。

## Risks

該当なし。
