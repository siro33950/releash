# Design

## The actual design

### Architecture

#### Workspace tree branch 行のアイコン選択

`src/components/workspace/WorkspaceList.tsx` の `WorkspaceBranchRow` が、`WorkspaceTreeItem.kind` に基づく合成 Node のアイコン選択を引き続き所有する。`fanout` 分岐は既存の `FanoutRowStatusIcon` を使用し、Sequence 分岐だけが `WorkspaceBranchStatusIcon` へ渡す Lucide アイコンを `ListTree` から `Waypoints` へ変更する。新しい component や Node 種別判定は追加しない。

`WorkspaceTreeItemRow` はトップレベルの項目にも各合成 Node の `children` にも再帰的に使われ、すべての Sequence を同じ `WorkspaceBranchRow` へ渡している。この既存経路を維持することで、一箇所のアイコン選択がトップレベルと入れ子の Sequence の双方へ適用される。根拠は `src/types/workspace-tree.ts` の `WorkspaceTreeItem` discriminated union と、`src/components/workspace/WorkspaceList.tsx` の `WorkspaceTreeItemRow` / `WorkspaceBranchRow` である。

主要な変更対象は次のとおり。

| Path | 変更の要旨 |
| --- | --- |
| `src/components/workspace/WorkspaceList.tsx` | `lucide-react` の import と Sequence 分岐の icon prop を `Waypoints` へ置き換える |
| `src/components/workspace/WorkspaceList.test.tsx` | Sequence の形状に関する既存観測を `Waypoints` へ更新し、トップレベル・入れ子・4状態分類・Fanout 維持を Behavior に対応づけて検証する |

状態分類の決定、色の対応、pulse の決定は変更しない。これらは Issue #1683 の設計どおり backend-owned classification と既存の `WorkspaceBranchStatusIcon` / `WorkflowNodeStatusIcon` が担い、本件は Node 種別を表す形状だけを変更する。

### Interface

公開 Tauri command、local API、CLI、DTO、TypeScript の公開型、component props は変更しない。`WorkspaceBranchStatusIcon` の内部 interface も維持し、既存の `LucideIcon` 型の icon prop に `Waypoints` を渡す。互換性のない契約変更はない。

### Data Model

該当なし。`WorkspaceSequence`、`WorkspaceFanout`、`WorkspaceTreeItem` および状態分類の保持方法を変更しない。

### Database

該当なし。永続化する事実、projection、SQLite schema、access path を変更しない。

### UI/UX

Sequence 行は `Waypoints` を `WorkspaceBranchStatusIcon` 経由で表示する。同 component の既定 `size-3.5` を使うため14pxを維持し、`workflowNodeIconClasses` による青・黄・赤・緑と、`isWorkspaceNodePulseStatus` による `active` / `attention` のみの pulse をそのまま適用する。

Fanout 行は既存の `FanoutRowStatusIcon` 経由で `GitFork` を表示し続ける。行の階層、インデント、展開・折り畳み、ラベル、操作領域は変更しない。

検証は既存の component test を拡張し、トップレベルと入れ子で共有される Sequence のアイコン選択、Sequence の既存表示規則、Fanout のアイコン維持を観測する。共有される色・pulse 規則については、既存の関連 component test を回帰検証として維持する。

### Algorithm

該当なし。

### Infra

該当なし。

## Alternatives Considered

該当なし。`Waypoints` は R-001 で確定済みであり、代替アイコンの再選定は Non-goal である。

## Cross-cutting concerns

該当なし。

## Risks

該当なし。
