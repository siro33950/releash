# Design

## The actual design

### Architecture

本設計は [`specs/unified-node-model/decisions.md`](../unified-node-model/decisions.md) の「Worktree 実行コンテキスト」「Worktree 出自の台帳と突合」「永続化」、`docs/architecture/DOMAIN.md`、`docs/architecture/USECASE.md`、`docs/architecture/GATEWAY.md`、および #1466 の実装である `src-tauri/src/domain/workflow/value_objects/node_fact.rs`、`src-tauri/src/domain/workflow/services/fact_replay.rs`、`src-tauri/src/adaptor/gateway/workflow/fact_log.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs` を根拠とする。

#### 責務 owner

worktree 出自の台帳は workflow domain が所有する。`node_events` の worktree 事実を tree fold へ取り込み、Node attempt ごとの隔離 worktree のライフサイクル、Git 実体との突合結果、管理 UI 向けの分類、resume 可否を一つの domain model から導出する。repository domain は Git が返す worktree／branch の実体だけを提供し、出自や削除可否を判定しない。

#1466 の `WorkflowRuntimeHost::reconcile_startup` と `fact_log::reconcile_tree_pass` を起動時突合の実行 owner として維持する。worktree 専用の復旧 entrypoint は作らず、既存 pass が domain の突合判断を適用してから、既存のプロセス喪失処理と未実行 advance を続ける。

永続正本は `node_events` だけとする。UI 用分類を保持する process-local projection は許容するが、事実ログから再構築できる summary に限定し、ファイル、別テーブル、snapshot 文書として永続化しない。

#### 主要な変更対象

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/node_fact.rs` | 隔離 worktree の生成・解放・喪失を純粋事実の語彙へ追加する |
| `src-tauri/src/domain/workflow/value_objects/worktree_origin.rs` | Node attempt が所有する台帳 entry、ライフサイクル、管理 UI 分類、隔離用 path／branch identity を定義する |
| `src-tauri/src/domain/workflow/services/fact_replay.rs` | worktree 事実を tree fold に取り込み、Node ごとの所有状態と recovery fence を導出する |
| `src-tauri/src/domain/workflow/services/worktree_reconciliation.rs` | 台帳、Node 状態、Git inventory から喪失記録・掃除候補・通常表示・非表示を決める純粋な突合規則を所有する |
| `src-tauri/src/domain/workflow/repository.rs` / `gateway.rs` | 事実ログ由来の台帳 snapshot 境界と、変更能力を持たない Git worktree inventory 境界を定義する |
| `src-tauri/src/adaptor/gateway/workflow/fact_log.rs` / `workflow_host.rs` | 同一 reconciliation pass 内で inventory を読み、必要な喪失事実を追記して再 fold した後に既存復旧を進める |
| `src-tauri/src/adaptor/gateway/workflow/worktree_gateway.rs` | 既存 repository 読み取りを `WorktreeInventoryGateway` へ変換する。create／remove／prune は公開しない |
| `src-tauri/src/adaptor/gateway/workflow/worktree_ledger_repository.rs` | `node_events` と domain 台帳の相互変換、および再構築可能な process-local summary を実装する |
| `src-tauri/src/usecase/repository_query_service.rs` / `repository_dto.rs` / `repository_state/service.rs` | raw Git read model に同じ domain 分類を request 時に重ね、worktree 一覧と branch card snapshot の分類を一致させる |
| `src/App.tsx` / `src/hooks/useWorktreeList.ts` / `src/components/workspace/WorkspaceList.tsx` / `src/types/git.ts` | backend 分類を通常一覧、非表示、掃除候補の表示へ写像する。出自判定は行わない |

`ExecutionStore::by_worktree` と `pending_resume_worktrees` は変更しない。同一 root worktree への2つ目の workflow 実行木の拒否を維持し、単独 Session と同一実行木内の Node に新しい worktree 排他を追加しない（R-009、B-009〜B-011）。`worktree` field の validation も変更せず、`WFU002` を維持する（R-010、B-012）。

### Interface

既存 Tauri command `list_worktrees`、`list_branches_with_status`、`list_branches_with_status_snapshot` の名前、入力、既存 response field は維持する。`WorktreeEntryDto` と、worktree を持つ `BranchCardDto` に `management_kind` を追加し、次の閉じた値を返す。

| Value | 意味 |
| --- | --- |
| `working_area` | 人間が作る作業の場。通常一覧へ表示する |
| `isolated_owned` | 台帳上で解放されておらず、所有 Node が再開対象になり得る隔離実行環境。管理 UI には表示しない |
| `cleanup_candidate` | 台帳上で解放済み、または所有 Node の実行が終了した隔離実行環境。「掃除候補」として通常一覧と分離する |
| `untracked_cleanup_candidate` | 台帳に有効な所有記録がなく、隔離用 path と branch の組に一致する実体。「台帳外・掃除候補」として通常一覧と分離する |

branch が worktree を持たない場合の `management_kind` は `null` とする。既存の Git read model と並び順は変更しない。field 追加は additive とし、workflow の `ManagedWorktreeGateway` は分類にかかわらず raw Git inventory から既存 worktree を解決できる。

隔離 worktree 喪失は既存 workspace tree interface へ写像する。対象 Node の `get_workspace_node_detail` にある `recoveryReason` と、実行木の `resumeUnavailableReason` に `isolated worktree is missing: <normalized path>` を設定する。既存 `resume_workflow_execution` は同じ typed cause を `InvalidState` として返し、`ResumeRequested` を追記せず、別 worktree で起動しない。controller や frontend は文字列解析で原因を再分類しない。

Node 単位の再実行も同じ判断に揃える。喪失を観測した Node の `capabilities.canRetry` は `false` とする。#1466 の起動時 pass が追記する `process_exited` は `failure_kind: None` であり、既存導出（`src-tauri/src/domain/workspace_tree/entities/mod.rs:784`）のままでは `canRetry` が `true` に残るため、喪失記録を持つ Node は導出側で retry 不可へ倒す。`retry_workspace_node` → `retry_node`（`src-tauri/src/usecase/workflow/workspace_node_command.rs:84-97`）は喪失 Node に対して resume と同じ typed cause を `WorkflowError::InvalidState` として返し、新しい attempt を開始しない（R-002、B-002）。

内部境界は次の2つを追加する。

| Interface | Responsibility |
| --- | --- |
| `IsolatedWorktreeLedgerRepository` | `node_events` から台帳 summary を復元し、worktree 事実を同じ table へ追記した後に process-local summary を更新する |
| `WorktreeInventoryGateway` | 設定済み repository ごとの normalized worktree path と branch の read-only snapshot を返す |

新しい Tauri command、WebSocket message、HTTP route、CLI、worktree 作成／取得／削除 interface は追加しない（R-007、R-008、B-007、B-008）。

### Data Model

台帳 entry の identity は `NodeFactMeta` が持つ `(tree_id, node_execution_id, attempt)` とする。生成事実は、突合に必要な normalized repository root、worktree path、branch 参照を保持する。同じ identity の解放・喪失事実を fold し、`created`、`released`、`lost` のライフサイクルを導出する。所有 Node の実行終了は既存 Node 状態から導出し、台帳 fact や status field として複製しない。

process-local 台帳 summary は repository root と worktree path で entry を引ける再構築可能な read model とする。保持するのは identity、Git 参照、worktree ライフサイクルだけで、Session 本文、Artifact、command output、実行木全体、Git inventory snapshot は保持しない。

隔離用 identity token は `<node_execution_id>-a<attempt>` とし、次の組を canonical rule とする。

- path: `worktrees_root(repo)/.releash-isolated/<identity token>`
- branch: `releash/isolated/<identity token>`

`worktrees_root(repo)` は既存 `src-tauri/src/domain/repository/value_objects/worktree_path.rs` の sibling worktree root（`<repo名>-worktrees`）を使うため、repository root 内へ worktree を置かない。フォールバック一致は、normalized path と branch の両方が同じ identity token で完全一致する場合に限る。片方だけの一致は台帳外・掃除候補へ分類しない。

### Database

既存 `node_events` table に次の `event_type` を保存する。

- `isolated_worktree_created`
- `isolated_worktree_released`
- `isolated_worktree_lost`

`event_type` は schema 上 CHECK を持たず domain 語彙が正本であるため、table、column、index、schema version の変更は行わない。生成事実の `detail` に Git 参照を保存し、解放・喪失は同じ Node attempt の row metadata へ紐づける。導出 status、管理 UI 分類、掃除候補、resume 不可理由は保存しない。

起動時は #1466 が既に行う tree 単位の `(tree_id, seq)` fold から台帳 summary を再構築する。再構築は、`reconcile_startup` が root 種別と `aggregate.is_active()` で木を落とす分岐（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:572-578`）より前に行い、完了・archive 済みを含むすべての木を対象にする。所有 Node の実行が終了した②は非 active な木に属するため、この順序でなければ掃除候補が summary から欠落する（R-003、B-003、B-013）。通常時の worktree 事実 append は `IsolatedWorktreeLedgerRepository` に集約し、durable append 成功後だけ process-local summary へ delta を適用する。append と summary 更新の間で process が停止しても、次回起動時に `node_events` から復元する。

### UI/UX

通常の worktree／branch card 一覧には `working_area` だけを従来どおり表示する。`isolated_owned` は表示しない。`cleanup_candidate` と `untracked_cleanup_candidate` は通常一覧の外に情報表示専用の掃除候補 section としてまとめ、それぞれ「掃除候補」「台帳外・掃除候補」と明示する。候補から実行木を開いたり自動選択したりしない。

`App.tsx` の初期 worktree 自動選択も `working_area` の件数だけで判定する。frontend は `management_kind` を表示先へ写像するだけで、path、branch、Node status から出自を再判定しない。

新しい cleanup 操作は追加しない。候補の表示によって `remove_worktree`、branch 削除、prune を自動発火させない。

### Algorithm

domain reconciliation は、同じ repository について取得した台帳 snapshot、fold 済み Node 状態、Git worktree inventory を一度に受け、次を判定する。

所有 Node の状態は、既存 `RuntimeNodeExecutionStatus::is_active()`（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:133-146`）を境界として2つに分ける。`Running` / `Paused` / `WaitingApproval` を「再開対象になり得る所有 Node」、`Succeeded` / `Failed` / `Aborted` を「実行が終了した所有 Node」とする。`Failed` は attempt が終了しており、retry は新しい attempt として別の隔離環境を生むため終了側に置く。

- 有効な生成記録があり、再開対象になり得る所有 Node の worktree 実体が無い場合は、未記録のときだけ `isolated_worktree_lost` を返す。
- 実体があり、entry が解放済み、または所有 Node の実行が終了している場合は `cleanup_candidate` を返す。
- 実体があり、entry が解放されておらず、所有 Node が再開対象になり得る場合は `isolated_owned` を返す。
- 有効な所有 entry が無い実体は canonical path／branch rule を両方満たす場合だけ `untracked_cleanup_candidate`、それ以外は `working_area` を返す。

起動時 pass は、Git inventory の取得が全件成功した repository だけを突合する。喪失事実を既存 single-row append で確定し、再 fold して recovery fence を反映した後に、プロセス喪失の観測と pending advance を行う。喪失した Node は leaf 起動対象から除外する。喪失事実の append に失敗した場合は、その木の leaf 起動へ進まず、既存 startup recovery の失敗として返す。同じ pass を再実行した場合、既存の `lost` fact があるため追加 row と外部作用は生じない。

台帳の read／decode に失敗した場合、起動時 reconciliation は対象をディスク形状だけで復旧せず、既存 startup recovery の失敗として再試行へ返す。管理 UI の query は canonical naming rule だけを defensive fallback として適用し、一致した実体を `untracked_cleanup_candidate` に隔離する（R-011、B-014、B-015）。Git inventory の取得に失敗した場合は「全 worktree が消えた」とみなさず、喪失事実を一切追記しない。

B-008 の非変更保証は、reconciliation が依存する `WorktreeInventoryGateway` を read-only capability に閉じることで構造的に保証する。受入検証では、実 Git repository を使う gateway test で3分岐と naming rule を確認し、domain／usecase test で lost fact の冪等性・resume と Node retry の拒否・通常一覧分類を確認し、component test で通常一覧／掃除候補／非表示を確認する。B-009〜B-012 は既存 start guard、ExecutionStore、validation の regression test を維持する。

### Infra

該当なし。既存 `git2` worktree 列挙、固定 `LocalEventStore`、起動時 recovery task を再利用し、新しい process、service、database、deploy component は追加しない。

## Alternatives Considered

- worktree 台帳用の table／JSON file を新設する案: `node_events` と二重の永続正本になり、R-001 と統一 Node モデルの永続化判断に反するため採らない。
- path または branch の片方だけでフォールバック判定する案: 人間が作った branch／directory の誤分類範囲を必要以上に広げ、Issue #1467 が要求する「専用パス + branch」の組を満たさないため採らない。
- repository gateway の `list` 自体から隔離 worktree を除外する案: workflow runtime の managed worktree 解決、GC、status scan まで実体を見失う。出自分離は管理 UI read model の関心なので、raw Git inventory は維持して query 時に分類する。
- worktree 専用の startup recovery を追加する案: #1466 の冪等 reconciliation と別の順序・retry authorityを作るため採らない。

## Cross-cutting concerns

該当なし。

## Risks

該当なし。
