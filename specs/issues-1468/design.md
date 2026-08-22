# Design

## The actual design

### Architecture

#### Worktree 配下の実行木 read model

`src-tauri/src/domain/workspace_tree/` が、`node_events` から Worktree 配下の実行木を構築する責務を引き続き所有する。実行開始済み Node の選別、親子関係、実行順、Node status、capability、および retry 履歴の分類はこの domain で確定し、frontend では再計算しない。

Worktree の既存 UI container の直下へ、各実行木の root を同じ `WorkspaceTreeItemDto` の再帰構造で並べる。workflow の実行木は定義 root（`main`）の実行インスタンスを root item とし、事実ログ上の workflow root は画面上の別 branch に変換しない。`main` が合成子なら root item は Sequence または Fanout branch、`main` が leaf Node なら root item は Node 1行になる。単独 Session の実行木は Session Node 1行を root item とする。これにより、単独 Session と workflow を別一覧へ分けず、かつ単独 Session のために実行事実にない Sequence を合成しない。

`src-tauri/src/adaptor/gateway/workspace_tree/repository.rs` は `TreeRootFact::Workflow` と `TreeRootFact::Session` の双方を同じ fold 入力として返す既存境界を維持する。`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs` は、Workflow root だけを選別する経路、Session root を `sessions` へ分離する経路、および Sequence を Fanout へ変換する経路を廃止し、domain が確定した再帰構造を公開 DTO へ写す。

内部境界 `WorkspaceQueryService` は、Tauri、WebSocket、および将来の client surface が共有する Workspace tree / Node detail read contract という既存責務を維持する。今回の tree 形状と retry 履歴はこの trait の `workspace_tree` 結果へ集約し、controller 固有の合成処理を設けない。

workflow 全体の stop / resume / abort / archive capability は、画面から除く workflow root の情報として失わず、公開 root item に関連付ける。owner は「公開 root item であること」で決まり、その item が Sequence / Fanout / Node のどれかには依存しない。したがって `main` が leaf Node の workflow では、その Node leaf が Node 自身の capability に加えて workflow 全体の capability を持つ。nested Sequence と Fanout は構造を束ねる branch に限り、Node detail の対象にしない。Session と Command は同じ Node leaf とし、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs` が返す detail だけが中央表示の入力になる。（R-001〜R-004、R-008、B-001〜B-004、B-009〜B-011）

#### retry 履歴の所有

同じ定義名や attempt 値の一致だけでは retry と loop を区別できないため、retry 履歴は `NodeFact::RetryRequested` と、それに対応して開始された次の Node の関係から domain が導出する。対応付けは `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` の replay が用いる「同じ Node 名かつ同じ親 scope」という restart 判定と同じ規則を使う。`RetryRequested` を伴わない同名 Node の再訪や Fanout の別展開は同じ retry 履歴へまとめない。

retry chain の最新行は通常の Node leaf として常に返し、それ以前の決着済み行は実行順の `past_attempts` として最新行へ関連付ける。公開 read model は過去行を省略せず、既定の表示状態が折り畳みであることも Rust で指定する。frontend は指定された順序と既定状態を描画し、利用者の展開状態だけを component state に保持する。（R-006、R-007、B-006〜B-008）

#### frontend の責務

`src/types/workspace-tree.ts`、`src/hooks/useWorkspaceTreeNodes.ts`、`src/components/workspace/WorkspaceList.tsx`、`src/screens/MainLayout.tsx`、および `src/components/panels/NodeContentView/` は、新しい backend DTO の mirror、branch と retry 履歴の開閉、Node 選択、command 呼び出しに限定する。active な単独 Session の取得・並べ替え・status/capability 判定を frontend に残さない。

最終的な中央選択は `CenterSelection.kind = "node"` に統一する。Session の新規作成中だけは既存の一時的な launching 表示を利用できるが、作成または restore の完了後は `get_workspace_session_node_id` で共通 Node ID を取得し、`NodeContentView` を表示する。`agent_session` 選択と `AgentSessionRoute` の直接表示経路は廃止する。（R-002、R-008、R-019、B-002、B-009、B-023）

#### 正本文書と example の所有

構文の公開正本は `docs/workflow-yaml-syntax.md` とし、`specs/unified-node-model/syntax.md` で確定した構文および `specs/issues-1591/` で確定した Lua 定義の契約を反映する。engine と Workspace のモデル説明は、次の既存正本文書をそれぞれの責務の範囲で改訂する。

- `docs/workflow-engine-evolution-plan.md`: Node 4種、completion、実行木全体の3値 status
- `docs/workflow-engine-model-boundary.md`: Worktree 配下の実行木と状態所有境界
- `specs/workflow-lifecycle/workflow-ideal-lifecycle.md`: 3値の木全体 status と Node 所有の詳細状態に基づく不変条件、遷移、操作受理、capability
- `docs/architecture/GLOSSARY.md`: 正規語、使用禁止語、および通常・隔離 Worktree の状態所有

example の実体は `specs/unified-node-model/examples/full-cycle-development.yml` を唯一の正本とする。`docs/examples/full-pipeline.yml` は削除し、docs 側は正本 example への参照だけを持つ。同ファイルの `worktree` 宣言は取り除く。`worktree` は `WFU002`（Error）で load が拒否され milestone #85 まで解禁されないため、宣言を残したままでは唯一の正本が load も実行もできない。loader と実行経路の検証も同じファイルを入力にし、別の同期用 example を作らない。（R-009〜R-017、B-012〜B-022）

#### 検証境界

Rust の domain / query service 検証では、Workflow と Session の両 root、nested Sequence、Fanout、実行開始済み Node だけの投影、retry と loop の区別、実行順、公開 root item へ移した workflow capability（root item が合成子の場合と leaf Node の場合の双方）、および archive 済み Session の除外を確認する。frontend の component 検証では、共通 Node 選択、branch の再帰表示、過去 attempt の既定折り畳みと展開、既存操作、および raw metadata 非表示を確認する。

example は唯一の正本ファイルを入力に、loader の Diagnostic がゼロであることを検証する。第2の永続 example も同期用の複製も作らない。実行経路も同じファイルを入力にし、外部 provider や process を起動せず、既存の domain engine 境界で nested Sequence と Fanout の子に置かれた合成子が開始・完了することを確認する。

正本間の検証では、対象文書、schema、Diagnostic、および example fixture の正規語、Node 種別、status、構文、example 参照を比較し、旧語 `gate`、6値 ExecutionStatus、旧3種 Node、旧 Workspace 並列構造、旧構文の暗黙変換、および削除した example への参照が残っていないことを確認する。（R-004〜R-007、R-009〜R-019、B-004〜B-008、B-012〜B-023）

### Interface

Tauri command `list_workspace_worktree_nodes(worktreePath)` と `get_workspace_tree_selection_reconciliation(worktreePath, selectedNodeId)` の名前と入力は維持する。両 command が返す `WorkspaceTreeSnapshotDto` は、active な実行を `nodes`、archive 済み単独 Session を `archivedSessions`、初期選択候補を `preferredNodeId` として返す。active / archive 済みを混在させた従来の `sessions` は削除する。

`nodes` の公開 union は `Node | Sequence | Fanout` とする。Sequence と Fanout は再帰的な children を持つ。workflow 実行木の公開 root item は Sequence / Fanout / Node のいずれにもなり得るため、workflow 全体の capability は item 種別ではなく公開 root item であることに紐づけ、3種のいずれでも保持できる形にする。公開 root item 以外の item は workflow 全体の capability を持たない。Node は `session | command` の content kind と Node 操作 capability を持つ。単独 Session Node には既存の archive / delete command へ渡す opaque な Session 参照と、Rust が導出した lifecycle capability を関連付ける。これらの参照は操作対象の指定にだけ使い、ラベルとして表示しない。

workflow 実行木の公開 root item の `id` は既存 workflow action の opaque な execution target を兼ね、それ以外の item の `id` は tree item の opaque identity とする。frontend はいずれも値を解析せず、公開 root item の workflow action 時だけ既存 workflow command へそのまま渡す。

retry 履歴を持つ Node は、実行順に並んだ過去の Node summary と既定の折り畳み指定を返す。最新行に置く履歴開閉操作によって、折り畳み時は最新行だけを、展開時は過去行を最新行の前へ実行順に表示する。履歴を表す実行事実のない tree item は追加しない。

`get_workspace_node_detail(worktreePath, nodeId)` は Session と Command の detail を同じ契約で返す。`WorkspaceNodeDetailDto.attempt`、重複した `AgentSession` content variant、および frontend の `queued` / `error` status は削除し、detail の Session content を1種類に統一する。内部 ID、attempt 番号、Fanout の item / child 座標は DTO の操作用 opaque 値として必要な場合も表示しない。（R-002、R-005〜R-008、R-018、R-019、B-002、B-005〜B-011、B-022、B-023）

`get_workspace_session_node_id`、単独 Session の create / archive / restore / delete、workflow の stop / resume / abort / archive、および Node の approve / retry / close command は維持する。操作の入力と成功・失敗の外部契約は変更せず、呼び出し元だけを統一 Node 行へ接続する。

### Data Model

実行木の正は引き続き `node_events` の事実列であり、Workspace tree は再構築可能な read model とする。永続化する新しい tree snapshot、status、retry group、または UI の開閉状態は追加しない。

公開する各実行行の identity は既存の opaque Node ID とする。retry 履歴は最新 Node が所有する一時的な `past_attempts` collection であり、各要素は既存 Node summary と同型とする。identity や並びは attempt 番号ではなく事実列から導出し、attempt 番号自体は公開表示モデルに保持しない。Session 本文、Artifact 本体、および Command output は tree summary に複製せず、Node detail が既存 owner への参照から必要時に取得する。

archive 済み単独 Session は active tree に含めず、既存 AgentSession lifecycle の summary と同型の `archivedSessions` が所有する。restore 後は同じ Session の実行木 Node が active tree に再び現れ、archive history から外れる。永続形式の versioning と既存データ移行は不要である。（R-001、R-004、R-006〜R-008、R-017、B-001、B-004、B-006〜B-009、B-021）

### Database

該当なし。既存の `node_events` と AgentSession lifecycle の access path を利用し、schema、index、projection table、および migration は追加しない。

### UI/UX

Worktree 行を開いた表示では、active な単独 Session と workflow 実行を同じ再帰 tree renderer へ渡す。workflow 実行木の root 行も含め、Sequence は Sequence branch、Fanout は Fanout branch、Session と Command は選択可能な leaf として表示する。root 行が leaf Node の場合も leaf として表示し、workflow 全体の操作をその行から提示する。Sequence / Fanout の開閉方法は既存 branch 操作を維持し、合成子を選択して中央表示を切り替える動作は追加しない。

retry がある Node は最新行に過去実行の開閉操作を持つ。初回表示では決着済みの過去行を隠し、展開すると過去行を実行順に表示する。各行の title と status は通常の Node 行と同じ情報に限り、`Attempt N` や代替の番号ラベルは付けない。

すべての Session / Command leaf は選択時に `NodeContentView` を使う。Node header から attempt 表示を削除し、内部 ID や展開座標も表示しない。単独 Session の archive / delete は統一 Node 行の capability に従って表示し、archive 済み Session の一覧と restore / delete は Worktree の既存 Session history menu に残す。（R-001〜R-008、R-018、R-019、B-001〜B-011、B-022、B-023）

### Algorithm

実行木の fold は tree ごとに一度だけ行い、その結果から画面用 summary を構築する。Workflow root では内部 root の子である実行開始済み `main` の実行インスタンスを公開 root とし、workflow capability をそこへ移す。`main` が leaf Node の場合も同じ規則で、その Node leaf が公開 root として workflow capability を持つ。Session root では同じ fact fold から root Session Node を公開する。いずれも定義上の children や expected slot を参照して未開始行を補完しない。

retry 履歴の導出では、対象 Node の `RetryRequested` と対応する次の start を chain として結び、chain 内の最後の実行を最新行、それ以前で決着済みの実行を `past_attempts` とする。この関係がない同名実行は、連続していても別 occurrence のままにする。これにより、実行順を維持しながら retry だけを既定折り畳みの対象にできる。（R-004、R-006、R-007、B-004、B-006〜B-008）

### Infra

該当なし。

## Alternatives Considered

### 単独 Session の一覧を tree と併存させる

既存 DTO と frontend 分岐を残せるが、同じ Worktree の実行を2系統へ分け、R-001、R-002、および R-019 を満たさないため採用しない。archive 済み Session の history だけは active tree とは別の既存操作面として維持する。

### frontend で Session と workflow の結果を結合する

frontend が順序、status、capability、retry 関係を判断することになり、backend-owned read model と複数 client で同じ状態を読む原則を満たさないため採用しない。

### retry 履歴用の仮想 branch 行を追加する

過去行を開閉しやすいが、実行事実のない行を実行木に追加して R-004 の表示契約を破るため採用しない。最新の実行行自体を開閉操作の owner とする。

### attempt 番号または同名 Node の連続性から retry を推定する

loop による再訪や Fanout の別展開を retry と誤分類するため採用しない。事実ログの retry 関係だけを使う。

### docs 用に canonical example を複製する

2つの定義を同期する必要が残り R-015 を満たさないため採用しない。`specs/unified-node-model/examples/full-cycle-development.yml` の実体1つを双方から参照する。

## Cross-cutting concerns

### 互換境界

Tauri command 名、入力、既存 mutation の結果は維持する。一方、`WorkspaceTreeSnapshotDto`、tree item union、Node detail、および `CenterSelection` は旧 UI 経路を残さず同じ変更で置き換えるため、同一リポジトリ内の Rust と frontend を一括更新する breaking change とする。旧 `sessions`、`Workflow` item、Sequence-to-Fanout 変換、`AgentSession` detail variant、`agent_session` selection、および廃止 status の alias や fallback は設けない。

### 保持量と機密情報

tree snapshot は構造、status、capability、時刻、および owner 参照だけを持つ。Session transcript、Artifact、Command output、provider credential、および raw command を retry 履歴を含む tree summary へ複製しない。過去 attempt の detail も選択された ID に対して既存 detail command で取得する。

### エラー境界

事実の decode、fold、または owner 参照の解決に失敗した場合は、`src-tauri/src/adaptor/gateway/workspace_tree/` から既存の typed error を `WorkflowError` まで保ち、Tauri command の既存エラー契約で返す。失敗した tree を旧 Session 一覧や定義から部分的に補完する fallback は行わない。既存 mutation の失敗分類は変更しない。

## Risks

該当なし。
