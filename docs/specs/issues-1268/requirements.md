# Requirements

Issue: #1268 「Workflow のステータスをアイコンで表現する」

関連: #1259 (Workflow/Step/Session ステータス集約) / #1242 (Workflow パネル再編) / #1220 (Workspace サイドバー 4 階層ツリー化)

## Type

WorkspaceList における **Workflow 行のステータス表示の見直し**（UI 表現の変更）。

## 背景と目的

GitHub Issue #1268 は本文が空のため、要求の確定はユーザーへのヒアリングに基づく（下記「合意済みの前提」参照）。

### 経緯

- #1220 以前の WorkspaceList では、Workflow 行は固定の `Workflow`（lucide）アイコン＋静的なミュート色 (`text-muted-foreground/80`) で表示されていた（形は常に同一で、Workflow という「種別」を表すアイコン）。
- #1259 で Workflow 行・Step 行のアイコンが `WorkflowStepStatusIcon` に置き換えられた。これはステータスごとに**アイコンの形そのものが変化する**（`running`→Loader2 / `completed`→CheckCircle2 / `failed`・`error`→AlertTriangle / `waiting`→Clock / `aborted`→Ban / `queued`→Circle）。
- 一方、Session 行のステータス表示 `AgentStateIcon` は、**常に同一の Bot アイコン**を用い、**色（および稼働中の pulse アニメーション）で状態を表現**している（`running`: info + pulse / `done`: success / `waiting`: warning + pulse / `error`: destructive）。

### 課題

- Workflow 行のアイコンは #1259 で「形が状態ごとに変わる」表現になったため、行が「Workflow である」という種別の識別性が失われた。状態によってアイコン形状が切り替わると、ツリー上で Workflow 行を一目で見分けにくい。
- Session 行（`AgentStateIcon`）とステータス表現の方式が不統一（Session は色で表現、Workflow は形で表現）であり、サイドバー全体の見た目に一貫性がない。

### 目的

WorkspaceList の Workflow 行を、**#1259 以前の固定 `Workflow` アイコンに戻した上で**、`AgentStateIcon` と同じ方式（同一形状のアイコンを保ち、**色**でステータスを表現、稼働中は pulse）で Workflow の代表ステータスを表現する。これにより「Workflow 行であること」の識別性とサイドバー全体のステータス表現の一貫性を両立させる。

## 合意済みの前提

- 本変更の対象は **WorkspaceList の Workflow 行のアイコン**に限定する（ユーザー確認済み）。
- 「元のアイコン」とは #1259 以前に Workflow 行で使われていた lucide の `Workflow` アイコンを指す。
- 表現方式は `AgentStateIcon`（Session 行）と同様、**単一アイコン形状＋色（＋稼働中の pulse）**とする。

## スコープ

- WorkspaceList の Workflow 行（`WorktreeWorkflowRow`）のステータスアイコンを、固定の `Workflow`（lucide）アイコンに戻す。
- 上記アイコンの**色**で Workflow の代表ステータス（`node.status`: `WorkspaceStepStatus`）を表現する。`AgentStateIcon` と同様に、稼働中（`running` 等）は pulse アニメーションで強調する。
- 表示に使う代表ステータス値は、既に Rust 側で導出・配信済みの `WorkspaceWorkflowNode.status`（および live 更新値）をそのまま用いる。
- 上記表現に必要なフロントエンドの表示用色マッピング（`WorkspaceStepStatus` → 色クラス）の定義。

## 非スコープ

- Workflow 行の **Step 行**（`WorktreeWorkflowStepRow`）のアイコン表現の変更。Step 行は現行の `WorkflowStepStatusIcon`（形＋色）を維持する。
- 中央パネル `WorkflowView` の Step ヘッダーのアイコン表現の変更。
- Session 行（`AgentStateIcon`）の表現変更。
- ステータスの**導出・集約ロジック**（Rust 側 `status_aggregation.rs` / `workspace_tree.rs` / `agent_session/status.rs`、優先順位、`WorkspaceStepStatus` の語彙）の変更。#1259 で確定したものをそのまま用いる。
- 新規 Tauri コマンド・WebSocket メッセージ・イベントの追加。
- Workflow engine の実行モデル・YAML schema・ナビゲーション構造の変更。

## 要求事項

### R1. Workflow 行アイコンを固定形状へ戻す

- WorkspaceList の Workflow 行のステータスアイコンは、ステータスによらず**常に同一形状**（lucide `Workflow` アイコン）で表示すること。
- これにより、ツリー上で Workflow 行が「Workflow である」ことを形状で一意に識別できること。

### R2. ステータスを色で表現する

- Workflow の代表ステータス（`WorkspaceStepStatus`）を、`Workflow` アイコンの**色**で表現すること。
- 色付けは `AgentStateIcon` と同じ方式に揃えること。すなわち:
  - 同一アイコン形状を保ったまま色のみでステータスを区別する。
  - 稼働中（`running`）など「処理が進行している」状態は pulse アニメーションで強調する（`AgentStateIcon` の `running` / `waiting` における `animate-pulse` に倣う）。
- 代表ステータスが未確定／不明な場合のフォールバック色を定めること（`AgentStateIcon` が `null` 時に `text-muted-foreground` を用いるのに倣う）。

### R3. 既存の代表ステータスをそのまま利用する

- 表示に用いるステータス値は、既に Rust 側で導出・配信されている `WorkspaceWorkflowNode.status` および live 更新値（`useWorktreeStepStatuses` 経由）を用いること。
- フロントエンドはステータスの導出・集約を行わず、受け取った代表ステータスを色へマッピングする表示用フォーマットのみを担うこと（rust-first-logic 準拠）。

### R4. 状態識別性の維持

- 各 `WorkspaceStepStatus`（`queued` / `running` / `failed` / `error` / `waiting` / `aborted` / `completed`）が、色（および pulse の有無）で相互に区別可能であること。
- ホバー時等に現在のステータスを確認できる手段（`title` 属性等）を維持すること（現行 `WorkflowStepStatusIcon` / `AgentStateIcon` が `title` を持つことに倣う）。

## 受け入れ基準の概要

- WorkspaceList の Workflow 行のアイコンが、全ステータスにおいて lucide `Workflow` アイコンの形状で表示される。
- Workflow の代表ステータスが変化すると、アイコンの形状は変わらず**色**（および pulse の有無）のみが変化する。
- `running` 等の稼働中ステータスで pulse アニメーションが適用される。
- 代表ステータスが live 更新（`workflow-step-status-changed`）された際、色がリアルタイムに追従する（既存の live 更新経路を踏襲）。
- Step 行・Session 行・WorkflowView のアイコン表現は本変更前後で変わらない。
- フロントエンドにステータス導出ロジックが追加されていない（色マッピングのみ）。
- `pnpm lint` / `pnpm test` / `pnpm build` が通る。

## 仮定

- A1. 色マッピングは、既存 `workflowStepIconClasses`（`WorkflowStepStatusIcon.tsx`）の配色をそのまま流用する（ユーザー確認済み）。すなわち `queued`→`text-muted-foreground` / `running`→`text-blue-600 dark:text-blue-300` / `failed`→`text-red-600 dark:text-red-300` / `error`→`text-destructive` / `waiting`→`text-yellow-600 dark:text-yellow-300` / `aborted`→`text-muted-foreground` / `completed`→`text-green-600 dark:text-green-300`。形状のみ `Workflow` アイコンへ固定し、配色は据え置く。
- A2. pulse を付与する対象は、`AgentStateIcon` に倣い「進行中／注目が必要」な状態とし、`running` と `waiting` に `animate-pulse` を適用する。
- A3. アイコンサイズ・コンテナのレイアウトは現行 Workflow 行の見た目（`size-5` コンテナ内に `size-3` 程度）を踏襲する。
- A4. Step 行は現行 `WorkflowStepStatusIcon` を維持し、本 Issue では変更しない（ユーザーが対象を「Workflow 行」と明示したため）。
- A5. 既存コンポーネント `WorkflowStepStatusIcon` 自体は Step 行・WorkflowView で引き続き使用されるため削除しない。Workflow 行用には新たな表示（例: `Workflow` アイコン＋色マッピング）を用意する想定。具体的な実装方式（新規コンポーネント化 or 既存への分岐追加）は design.md で確定する。

## Open Questions

なし。
