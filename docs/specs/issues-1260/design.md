# Design

Issue #1260 / `WorkspaceList` の Worktree 展開ノード配下が空のときの明示表示。

requirements.md (R1〜R4) と behavior.md (Gherkin) を満たす実装方針を定義する。

## 概要

`WorkspaceList` の Worktree 展開ノード配下に、Session / Workflow が 1 件も存在しない場合の空状態プレースホルダ表示を追加する。

`src/components/workspace/WorkspaceList.tsx` の `WorktreeTreeItem` コンポーネント内、展開時の配下描画ブロック (現状 839〜896 行付近) に、`treeLoading` / `treeError` のいずれでもない正常完了かつノード 0 件のときに限り、控えめなスタイルのプレースホルダ `No sessions or workflows` を描画する分岐を加える。

ロジックは「受け取ったデータの表示条件判定 (件数が 0 か)」に該当し、フロントエンドで扱ってよい (rust-first-logic の許可範囲)。新たな Tauri コマンド・バックエンド変更は不要 (requirements 仮定 A3)。

## 変更対象

| ファイル | 変更内容 |
| --- | --- |
| `src/components/workspace/WorkspaceList.tsx` | `WorktreeTreeItem` の配下描画ブロックに空状態プレースホルダの分岐を追加 |
| `src/components/workspace/WorkspaceList.test.tsx` | 空状態 / 1 件以上 / 読み込み中 / エラー / 折りたたみ の表示分岐をカバーするテストを追加 |

上記以外 (`useWorkspaceTreeNodes` 等のフック、Rust 側、他階層の空表示) は変更しない (requirements 非スコープ)。

## アーキテクチャと責務分割

- **状態の供給元**: `useWorkspaceTreeNodes(branch.worktree_path)` が返す `nodes` / `loading (treeLoading)` / `error (treeError)` を唯一の情報源とする。
- **表示用ノード**: 既存の `displayNodes` (live status を反映した `nodes` の派生) を空判定の対象とする。`nodes` ではなく `displayNodes` を判定対象にするのは、描画対象とプレースホルダ判定対象を一致させ「描画 0 件なのにプレースホルダが出ない」ズレを防ぐため。`displayNodes` は `nodes` を `map` した結果であり件数は常に一致する (仮定 B1)。
- **責務分割**: 表示分岐の判定・描画は `WorktreeTreeItem` 内に閉じる。プレースホルダ専用の子コンポーネントは新設せず、既存の `treeLoading` / `treeError` 分岐と同じ階層に並べる (見た目・インデントの一貫性を保つため)。

### 表示分岐の優先順位

展開済み (`expanded && hasWorktree`) の配下は、次の排他的優先順位で 1 つの状態のみを描画する (behavior の Scenario Outline に対応)。

1. `treeLoading` が真 → スピナー (既存)
2. `treeError` が真かつ `nodes.length === 0` → エラー表示 (既存)
3. 上記いずれでもなく `displayNodes.length === 0` → **空状態プレースホルダ (新規)**
4. 上記いずれでもない → ノード一覧 (既存)

現状コードは 1・2 を満たさない場合に常に 4 (`displayNodes.map(...)`) へ落ちるため、`displayNodes` が空のときは何も描画されない。本変更は 4 の手前に 3 を挿入する。

折りたたみ時 (`expanded` が偽) は配下ブロック自体が描画されないため、プレースホルダも当然描画されない (behavior「折りたたみ時は配下を一切描画しない」を既存構造で満たす)。

## データモデルまたは型

新規の型・データ構造は追加しない。既存の `UseWorkspaceTreeNodesResult` (`nodes` / `loading` / `error`) と `displayNodes: WorkspaceWorkflowNode | WorkspaceSessionNode[]` をそのまま利用する。

## 処理フロー

```text
WorktreeTreeItem render
  └─ expanded && hasWorktree ?
       ├─ treeLoading            → <Loader2 /> (スピナー)
       ├─ treeError && nodes==0  → エラー表示
       ├─ displayNodes.length==0 → "No sessions or workflows" (新規)
       └─ else                   → displayNodes.map(node => Workflow/Session 行)
  (workflowActionError は上記いずれの分岐でも従来どおり末尾に追記)
```

プレースホルダ要素 (案):

```tsx
<div
  className="truncate py-1 text-xs text-muted-foreground"
  style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
>
  No sessions or workflows
</div>
```

- `text-muted-foreground` + `text-xs`: 既存の空表示 (`No Repository`) およびエラー表示と同系統の控えめなスタイル (R3 / behavior「控えめな配色」)。
- `paddingLeft: WORKTREE_NAME_INDENT_PX`: 既存の `treeLoading` / `treeError` 分岐と同じインデント定数を用い、配下ノードと同等の字下げにする (behavior「同等のインデント」)。
- 文言は `No sessions or workflows` (requirements 仮定 A1)。

`workflowActionError` の表示 (886〜894 行) は配下分岐の外側にあり、空状態分岐を追加しても従来どおり描画される。空状態と `workflowActionError` は独立に共存し得る (空状態時に過去の archive/stop が失敗していればエラーも併記される) が、これは既存挙動の踏襲であり本変更では変えない。

## エラー処理

- 新規のエラーパスは発生しない (表示分岐の追加のみ)。
- `treeError` が真でも `nodes.length > 0` の場合は現状どおり分岐 2 を素通りし、ノード一覧 (分岐 4) を表示する。この場合は空状態プレースホルダの対象外 (`displayNodes` が非空)。本変更はこの既存挙動を変更しない。
- 「エラーかつ 0 件」は分岐 2 で捕捉され、分岐 3 (空状態) には到達しない (R2 / behavior 該当 Scenario)。

## テスト方針

`WorkspaceList.test.tsx` に以下を追加する。既存テストのモック方式 (`vi.mock("@/hooks/useWorkspaceTreeNodes")` が `worktreePath` 別に状態を返す) を踏襲し、状態ごとに異なる `worktreePath` を持つ branch を `useWorktreeList` モックから供給するか、または既存の `useWorkspaceTreeNodes` モックを `worktreePath` → 状態のマップで切り替える形に拡張する (仮定 B2)。

検証する振る舞い (behavior の各 Scenario に対応):

1. **完了かつ 0 件**: `nodes: []`, `loading: false`, `error: null` の Worktree を展開 → `No sessions or workflows` が表示され、Session 行・Workflow 行が 1 つも無い。
2. **完了かつ 1 件以上**: 既存の `/repo/wt` (ノードあり) を展開 → ノード一覧が表示され `No sessions or workflows` は表示されない。
3. **読み込み中**: `loading: true` → スピナー (`Loader2`) が表示され `No sessions or workflows` は表示されない。
4. **エラーかつ 0 件**: `error: "..."`, `nodes: []` → エラー文言が表示され `No sessions or workflows` は表示されない。
5. **折りたたみ時**: Worktree ノードを折りたたむ (展開トグルをクリック) → `No sessions or workflows` を含め配下が描画されない。

- スピナーは aria/role を持たないため、`Loader2` の描画は class などで確認するか、既存テストの判定手法に合わせる (仮定 B3)。
- 実行コマンド: `pnpm lint` / `pnpm test` (CLAUDE.md / CI 準拠)。

## リスクと代替案

- **リスク 1 (判定対象の不一致)**: `nodes` と `displayNodes` で件数判定がずれると、描画は空なのにプレースホルダが出ない/逆が起きる。→ 判定対象を描画対象と同じ `displayNodes` に統一して回避 (仮定 B1)。
- **リスク 2 (エラーかつ非空との競合)**: `treeError` かつ `nodes.length > 0` のケースは分岐 2 を通らずノード一覧を出す既存挙動。本変更はこの分岐を触らず、空状態は `displayNodes` が空のときのみ追加されるため競合しない。
- **代替案 A (プレースホルダ子コンポーネント化)**: `WorktreeEmptyState` のような専用コンポーネントを新設する案。再利用予定が無く、既存の `treeLoading` / `treeError` がインライン `div` で書かれていることとの一貫性から、本設計ではインライン追加を採用する。
- **代替案 B (空判定をフック側で算出)**: `useWorkspaceTreeNodes` に `isEmpty` を持たせる案。表示条件のための派生値であり、フックの責務 (データ取得) を超えるため不採用。表示側で `displayNodes.length === 0` を直接評価する。

## 仮定

- **A1 (文言)**: プレースホルダ文言は `No sessions or workflows` (requirements / behavior 仮定 A1)。
- **A2 (対象範囲)**: 対象は Worktree 展開ノード配下のみ。`RepoTreeSection` のブランチ一覧は対象外 (requirements 仮定 A2)。
- **A3 (バックエンド変更なし)**: 既存フックが返す `nodes` / `loading` / `error` で判定可能、Rust 変更不要 (requirements 仮定 A3)。
- **B1 (判定対象)**: 空判定は `displayNodes.length === 0` で行う。`displayNodes` は `nodes` の `map` 結果のため件数は `nodes` と常に一致する。
- **B2 (テストのモック拡張)**: 空状態テストは、空ノードを返す `worktreePath` を持つ branch を追加するか `useWorkspaceTreeNodes` モックを状態マップ化して供給する。既存テスト (`/repo/wt`) の挙動は変更しない。
- **B3 (スピナー判定)**: 読み込み中の検証は `Loader2` (animate-spin) の描画有無で行う。既存テストの判定手法に合わせる。

## Open Questions

なし。
