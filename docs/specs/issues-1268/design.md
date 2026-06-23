# Design

Issue: #1268 「Workflow のステータスをアイコンで表現する」

本書は `requirements.md` / `behavior.md` を実装方針へ落とし込む設計文書である。対象は **WorkspaceList の Workflow 行（`WorktreeWorkflowRow`）のステータスアイコン表現**に限定する。

## 概要

WorkspaceList の Workflow 行は現在 `WorkflowStepStatusIcon`（ステータスごとに**形状が変わる**: Loader2/CheckCircle2/AlertTriangle/Clock/Ban/Circle）で描画されている。これを **#1259 以前の固定 `Workflow`（lucide）アイコン**へ戻し、ステータスは `AgentStateIcon`（Session 行）と同じ方式 ── **同一形状＋色（＋稼働中の pulse）** ── で表現する。

これにより、

- Workflow 行が「Workflow である」ことを形状で一意に識別できる（R1）。
- Session 行と Workflow 行のステータス表現方式が「色で表す」方式に統一される（R2）。

ステータスの導出・集約・配信は Rust 側の既存実装（`WorkspaceWorkflowNode.status` および live 更新）をそのまま利用し、フロントエンドは**色クラスへのマッピングという表示用フォーマットのみ**を担う（R3 / rust-first-logic 準拠）。

## 変更対象

| ファイル | 変更内容 | 区分 |
| --- | --- | --- |
| `src/components/workspace/WorkflowRowStatusIcon.tsx` | **新規作成**。固定 `Workflow` アイコン＋色＋pulse＋title を描画するコンポーネント。 | 追加 |
| `src/components/workspace/WorkflowRowStatusIcon.test.tsx` | **新規作成**。新規コンポーネントの単体テスト。 | 追加 |
| `src/components/workspace/WorkspaceList.tsx` | `WorktreeWorkflowRow`（Workflow 行）の `WorkflowStepStatusIcon` 呼び出しを `WorkflowRowStatusIcon` に差し替え。import 追加。 | 変更 |
| `src/components/workspace/WorkflowStepStatusIcon.tsx` | `workflowStepIconClasses`（既に `export` 済み）を新規コンポーネントから再利用。**本体は変更しない**（Step 行・WorkflowView で継続使用）。 | 参照のみ |

### 変更しない箇所（非スコープの担保）

- `WorktreeWorkflowStepRow`（Step 行, `WorkspaceList.tsx:366`）の `WorkflowStepStatusIcon` 呼び出し。
- `WorkflowView.tsx`（中央パネル）の Step ヘッダー / `AgentStateIcon`。
- `WorktreeSessionRow`（Session 行, `WorkspaceList.tsx:209`）の `AgentStateIcon`。
- Rust 側のステータス導出・集約（`status_aggregation.rs` 等）、`WorkspaceStepStatus` の語彙、Tauri コマンド・WS メッセージ・イベント。
- live 更新経路（`useWorktreeStepStatuses` / `applyLiveWorkflowStatuses` / `workflow-step-status-changed`）。

## アーキテクチャと責務分割

### 設計判断: 新規コンポーネント方式を採用する（requirements A5 の確定）

requirements A5 が design へ委ねた「新規コンポーネント化 or 既存への分岐追加」は、**新規コンポーネント `WorkflowRowStatusIcon` の作成**で確定する。

- 採用理由:
  - `WorkflowStepStatusIcon`（形で表す）と `WorkflowRowStatusIcon`（色で表す）は表現方式が本質的に異なる。1 コンポーネントに分岐フラグを足すと、両方式が 1 箇所に同居し責務が曖昧になる。
  - Step 行・WorkflowView が使う `WorkflowStepStatusIcon` の挙動を一切変えず据え置ける（非スコープ担保が明快）。
  - `AgentStateIcon` が Session 行専用の小コンポーネントとして独立しているのと対称的で、サイドバーのアイコン構成が把握しやすい。
- 配置: `WorkspaceStepStatus` を扱うため、`WorkflowStepStatusIcon.tsx` と同じ `src/components/workspace/` に置く（`AgentStateIcon` は汎用 UI のため `ui/` だが、本コンポーネントは workspace ドメインに固有）。

### 各層の責務

- **Rust（変更なし）**: 代表ステータスの導出・集約・配信。`WorkspaceWorkflowNode.status` と live 更新値の供給源。
- **`useWorktreeStepStatuses` / `applyLiveWorkflowStatuses`（変更なし）**: live 更新を `node.status` へ反映し `WorktreeWorkflowRow` へ渡す。
- **`WorktreeWorkflowRow`（呼び出し差し替えのみ）**: `node.status` を `WorkflowRowStatusIcon` へ渡す。
- **`WorkflowRowStatusIcon`（新規）**: 受け取った `WorkspaceStepStatus` を色クラス＋pulse の有無＋title へマッピングし、固定 `Workflow` アイコンを描画する**表示用フォーマット責務のみ**。導出・集約・優先順位判定は行わない。

## データモデルまたは型

新規の型・WS メッセージ・Tauri コマンドは追加しない。既存型をそのまま利用する。

- `WorkspaceStepStatus`（`src/types/workspace-tree.ts`）: `"queued" | "running" | "failed" | "error" | "waiting" | "aborted" | "completed"`。
- `WorkspaceWorkflowNode.status: WorkspaceStepStatus`（同上）。

### `WorkflowRowStatusIcon` の Props

```tsx
interface WorkflowRowStatusIconProps {
    status: WorkspaceStepStatus;
    containerClassName?: string;
    iconClassName?: string;
}
```

- `circleClassName` は持たない（`queued` 用 `Circle` 形状を使わず、形状は常に `Workflow` で固定のため）。
- 呼び出し側（`WorktreeWorkflowRow`）は **Session 行（`AgentStateIcon`）と同一のアイコン配置に揃える**。すなわち `containerClassName` を渡さず（ラップ `span` はサイズ指定なし）、既定の `iconClassName="size-3.5 shrink-0"` をそのまま用いる（仮定 A3 改訂）。
  - 当初は旧 `WorkflowStepStatusIcon`（Step 行）のレイアウト（`containerClassName="flex size-5 shrink-0 items-center justify-center"` / `iconClassName="size-3"`）を踏襲したが、Workflow 行はサイドバー上で Step 行ではなく **Session 行と縦に並ぶ**。`AgentStateIcon` は 14px (`size-3.5`) アイコンをサイズ指定なしの `span` で左端に置くのに対し、size-5 (20px) ボックス中央寄せ＋12px アイコンでは左余白とボックス幅の差でアイコン・ラベルが右へずれる。R2（Session 行と表現方式を統一）の観点からも `AgentStateIcon` に揃えるのが正しい。

### 色・pulse マッピング

- **色**: `WorkflowStepStatusIcon.tsx` が `export` 済みの `workflowStepIconClasses: Record<WorkspaceStepStatus, string>` を **import して再利用**する（仮定 A1。配色定義を二重に持たない）。
  - `queued`→`text-muted-foreground` / `running`→`text-blue-600 dark:text-blue-300` / `failed`→`text-red-600 dark:text-red-300` / `error`→`text-destructive` / `waiting`→`text-yellow-600 dark:text-yellow-300` / `aborted`→`text-muted-foreground` / `completed`→`text-green-600 dark:text-green-300`。
- **pulse**: `running` と `waiting` にのみ `animate-pulse` を付与する（仮定 A2、`AgentStateIcon` に倣う）。コンポーネント内に対象集合を持つ。

  ```tsx
  const pulseStatuses: ReadonlySet<WorkspaceStepStatus> = new Set(["running", "waiting"]);
  ```

- **フォールバック色（仮定 A6）**: `status` が既知値に該当しない場合は `text-muted-foreground`、pulse なし。`AgentStateIcon` の `null` 時に倣う。
  - 型上は `WorkspaceStepStatus` で網羅されるが、`workflowStepIconClasses[status]` の参照結果が未定義となる事態（将来の語彙追加・実行時の想定外値）に備え、`workflowStepIconClasses[status] ?? "text-muted-foreground"` の形でフォールバックする。これは導出ではなく表示用フォーマットの防御的既定値。

## 処理フロー

1. Rust が代表ステータスを導出・配信 → `WorkspaceWorkflowNode.status`。
2. `useWorktreeStepStatuses` が `workflow-step-status-changed` を購読し、`applyLiveWorkflowStatuses` が `node.status` を最新値へ上書きして `displayNodes` を生成。
3. `WorktreeWorkflowRow` が `node.status` を `WorkflowRowStatusIcon` の `status` へ渡す。
4. `WorkflowRowStatusIcon` が以下を計算して描画:
   - 色クラス = `workflowStepIconClasses[status] ?? "text-muted-foreground"`
   - pulse = `pulseStatuses.has(status)` のとき `animate-pulse`
   - `title = status`（ホバー時にステータス確認、R4）
   - 形状 = 常に lucide `Workflow`
5. live 更新でステータスが変わると `displayNodes` 再計算 → 再レンダリングされ、**形状は `Workflow` のまま、色と pulse のみ追従**する。

### 描画構造（概略）

```tsx
import { Workflow } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceStepStatus } from "@/types/workspace-tree";
import { workflowStepIconClasses } from "./WorkflowStepStatusIcon";

const pulseStatuses: ReadonlySet<WorkspaceStepStatus> = new Set(["running", "waiting"]);

export function WorkflowRowStatusIcon({
    status,
    containerClassName,
    iconClassName,
}: WorkflowRowStatusIconProps) {
    const colorClass = workflowStepIconClasses[status] ?? "text-muted-foreground";
    const pulse = pulseStatuses.has(status) ? "animate-pulse" : undefined;
    return (
        <span title={status} className={containerClassName}>
            <Workflow className={cn(iconClassName, colorClass, pulse)} />
        </span>
    );
}
```

（既定サイズは `AgentStateIcon` と揃え `iconClassName="size-3.5 shrink-0"`。`WorktreeWorkflowRow` は `containerClassName`/`iconClassName` を渡さず既定のまま用いるため、ラップ `span` はサイズ指定なし・アイコンは 14px となり Session 行と一致する。）

## エラー処理

- 本変更は表示専用で I/O・非同期処理を持たないため、新規のエラー型・例外処理は不要。
- 想定外の `status` 値は前述のフォールバック色（muted・pulse なし）で安全に描画する（クラッシュさせない）。

## テスト方針

CLAUDE.md のテスト配置に従い、コンポーネントと同階層に `*.test.tsx` を置く。

### 新規 `WorkflowRowStatusIcon.test.tsx`（単体テスト）

`behavior.md` の Rule を網羅する:

- **形状一定（R1）**: 全 `WorkspaceStepStatus` 値で描画し、lucide `Workflow` アイコン（`svg.lucide-workflow` クラス）が表示され、Loader2/CheckCircle2/AlertTriangle/Clock/Ban/Circle が現れないこと。
- **色マッピング（R2）**: 各ステータスで `workflowStepIconClasses` に対応する色クラスが付与されること（render 結果の className を検証）。
- **pulse（仮定 A2）**: `running` / `waiting` で `animate-pulse` が付与され、`queued`/`failed`/`error`/`aborted`/`completed` で付与されないこと。
- **フォールバック（仮定 A6）**: 既知値以外（実行時の想定外値）を渡したとき `text-muted-foreground`・pulse なしになること。
- **title（R4）**: `title={status}` が提示されること（`getByTitle`）。

### 既存 `WorkspaceList.test.tsx`（統合テスト・確認/最小追記）

- Workflow 行が `Workflow` アイコン形状で描画されることの確認を追加。
- 既存の live 代表ステータス検証（`useWorktreeStepStatuses` モックで `workflows: Map([["run-1","failed"]])`、`getAllByTitle("failed")` 等）が、アイコン差し替え後も通ることを確認。title 属性は新コンポーネントでも維持するため既存アサーションは原則そのまま通る見込み。
- Step 行（`WorkflowStepStatusIcon`）・Session 行（`AgentStateIcon`）の表現が不変であることを担保する既存テストを壊さない。

### 既存テストへの影響確認

- Step 行は title=ステータス、Workflow 行も title=ステータスで、形状以外の DOM テキスト/title 構造は維持されるため、title ベースの既存アサーションへの破壊的影響はない見込み。実装時に `WorkspaceList.test.tsx` を実行して確認する。

### 品質チェック

- `pnpm lint` / `pnpm test` / `pnpm build` を通す（受け入れ基準）。

## リスクと代替案

- **リスク 1: 既存 `WorkspaceList.test.tsx` のアイコン関連アサーション破壊**
  - 既存テストは title 属性（`getAllByTitle`）で検証しており、形状クラスに依存していないため影響は小さい見込み。実装時にテスト実行で確証する。
- **リスク 2: `workflowStepIconClasses` の import による結合**
  - Workflow 行アイコン（色のみ）と Step 行アイコン（形＋色）が同じ配色定義を共有する。これは仮定 A1（配色流用）の意図そのもので、配色変更時に両者を一括で揃えられる利点となる。将来 Workflow 行だけ配色を変えたくなった場合は、その時点で定義を分離する。
- **代替案 A（不採用）: `WorkflowStepStatusIcon` に `variant`/`fixedShape` プロップを足して分岐**
  - 1 コンポーネントに「形で表す」「色で表す」両方式が同居し責務が肥大化。Step 行・WorkflowView への波及リスクも増えるため不採用。
- **代替案 B（不採用）: `AgentStateIcon` を汎用化して Workflow にも流用**
  - `AgentState`（`running|done|error|waiting`）と `WorkspaceStepStatus`（7 値）は語彙が異なり、アイコン形状も Bot↔Workflow で別。汎用化は両者の差異を吸収する分岐を生むだけで利得が薄く、非スコープ（Session 行不変）にも抵触しうるため不採用。

## 仮定

requirements / behavior の仮定（A1〜A6）を踏襲する。本設計で追加・確定した仮定は以下。

- **D1**: requirements A5 の未確定点を「新規コンポーネント `WorkflowRowStatusIcon` 作成」で確定する（上記「設計判断」参照）。
- **D2**: 新規コンポーネントは `src/components/workspace/` に配置する（`WorkspaceStepStatus` を扱う workspace ドメイン固有のため）。
- **D3**: 配色定義は `WorkflowStepStatusIcon.tsx` の `export` 済み `workflowStepIconClasses` を import 再利用し、定義を二重化しない。
- **D4**: コンポーネント名は `WorkflowRowStatusIcon`（`WorkflowStepStatusIcon` と並列・対比が明確な命名）。実装時に既存命名規約と齟齬があれば調整しうる。

## Open Questions

なし。
