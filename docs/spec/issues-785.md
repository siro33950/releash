## 要求

**種別**: 新機能
**ゴール**: Reviewパネル上部に、左右分割で差分ファイル一覧と差分表示を配置する
**背景**: #784で旧レイアウト（Editorタブ内のDiff表示・トップタブ・サイドパネル・コミットUI）を廃止した。Diff機能をReviewパネルという新しい専用ビューとして再構成する

**制約**:
- 全てのロジック（diff計算、ファイル一覧取得、stage/unstage操作等）はRust側で実装し、フロントはUIの表示とinvoke呼び出しに徹する
- gutterモードは独自のdiff表示コンポーネント、inline/splitモードはMonaco Diff Editorを使用する
- #784で保持されたDiff関連のフロントエンド（コンポーネント・hooks・utils）およびRustコマンドを流用する

**影響範囲**: Reviewパネルの新設。DiffBase型から旧`"staged"`を廃止し`"head"`に統一（設定マイグレーション含む）。既存のRightSidebarBottom（terminal/commentsタブ）は#786の対象であり本Issueのスコープ外

## 振る舞い定義

```gherkin
Feature: Reviewパネル
  左右分割で差分ファイル一覧と差分表示を配置するパネル。
  ブランチベースまたはHEAD基準の差分を閲覧し、hunk/group単位でstage/unstage操作を行う。

  Rule: Reviewパネルは左側にファイル一覧、右側に差分表示を配置する
    Scenario: Reviewパネルのレイアウト
      Given Reviewパネルが表示されている
      When ユーザーがパネルを確認する
      Then 左側に差分ファイル一覧が表示されている
      And 右側に選択ファイルの差分が表示されている

  Rule: 差分基準はbranch-baseとheadを切り替えられる
    Scenario: branch-baseからheadに切り替える
      Given 差分基準がbranch-baseである
      When ユーザーが差分基準をheadに切り替える
      Then 差分基準がheadに変わる
      And ファイル一覧と差分表示がhead基準で更新される

    Scenario: headからbranch-baseに切り替える
      Given 差分基準がheadである
      When ユーザーが差分基準をbranch-baseに切り替える
      Then 差分基準がbranch-baseに変わる
      And ファイル一覧と差分表示がbranch-base基準で更新される

  Rule: ファイル一覧はファイルツリーで表示される
    Scenario: 変更ファイルのツリー表示
      Given リポジトリに変更されたファイルがある
      When ユーザーがファイル一覧を確認する
      Then 変更ファイルがディレクトリ構造のツリー形式で表示されている
      And 各ファイルには変更ステータスが表示されている

  Rule: ファイルを選択すると差分が表示される
    Scenario: ファイル選択による差分表示
      Given ファイル一覧に変更ファイルが表示されている
      When ユーザーがファイルを選択する
      Then 右側に選択ファイルの差分が表示される

  Rule: 差分表示モードはgutter・inline・splitを切り替えられる
    Scenario: 差分表示モードの切り替え
      Given 差分が表示されている
      When ユーザーが差分表示モードを変更する
      Then 選択したモードで差分が再表示される

  Rule: hunk間をナビゲーションできる
    Scenario: 次のhunkへ移動する
      Given 複数のhunkを含む差分が表示されている
      When ユーザーが次のhunkへ移動する
      Then 次のhunkの位置にスクロールする

    Scenario: 前のhunkへ移動する
      Given 複数のhunkを含む差分が表示されている
      When ユーザーが前のhunkへ移動する
      Then 前のhunkの位置にスクロールする

  Rule: head基準ではhunk/group単位でstage/unstage操作ができる
    Scenario: グループ単位でstageする
      Given 差分基準がheadである
      And ワーキングツリーに未ステージの変更がある
      When ユーザーがグループをstageする
      Then そのグループの変更がステージされる

    Scenario: グループ単位でunstageする
      Given 差分基準がheadである
      And ステージ済みの変更がある
      When ユーザーがグループをunstageする
      Then そのグループの変更がアンステージされる

    Scenario: 全変更を一括stageする
      Given 差分基準がheadである
      And 未ステージの変更がある
      When ユーザーが全変更をstageする
      Then 全ての変更がステージされる

    Scenario: 全変更を一括unstageする
      Given 差分基準がheadである
      And ステージ済みの変更がある
      When ユーザーが全変更をunstageする
      Then 全ての変更がアンステージされる

  Rule: branch-base基準ではstage/unstage操作は提供されない
    Scenario: branch-base基準でのツールバー表示
      Given 差分基準がbranch-baseである
      When ユーザーがツールバーを確認する
      Then stage/unstageの操作ボタンは表示されていない

  Rule: 画像ファイルは専用の差分ビューアーで表示される
    Scenario: 画像ファイルの差分表示
      Given 変更ファイルに画像ファイルが含まれている
      When ユーザーが画像ファイルを選択する
      Then 変更前と変更後の画像が並列表示される

  Rule: 変更ファイルがない場合はその旨を表示する
    Scenario: 変更なし時の表示
      Given リポジトリに変更がない
      When ユーザーがReviewパネルを表示する
      Then 変更がない旨が表示される
```

## 実装仕様

**対応方針**: Reviewパネルを右パネル上部に新設し、内部を左右分割（ファイルツリー | 差分表示）で配置する。既存のDiff関連コンポーネント・hooks・utils・Rustコマンドを最大限流用する。ファイルツリー構築ロジックはRust側に新規コマンドとして実装し、フロントは受け取ったツリーデータの描画に徹する。

**対象コンポーネント**:

| 変更/新規 | ファイル | 変更内容 |
|----------|---------|---------|
| 新規 | `src-tauri/src/git/diff_tree.rs` | ファイルパス一覧からディレクトリツリー構造を構築するロジック。単一子ディレクトリの自動結合を含む |
| 変更 | `src-tauri/src/git/commands.rs` | `build_diff_file_tree` コマンドを追加 |
| 変更 | `src-tauri/src/git/mod.rs` | `diff_tree` モジュールを追加 |
| 変更 | `src-tauri/src/lib.rs` | `build_diff_file_tree` をinvokeハンドラーに登録 |
| 新規 | `src/components/panels/ReviewPanel.tsx` | Reviewパネル本体。左右分割（ファイルツリー \| 差分表示）を`react-resizable-panels`で構成 |
| 新規 | `src/components/panels/DiffFileTree.tsx` | Rust側で構築済みのツリーデータを描画。折りたたみUI・ファイル選択のみ担当 |
| 新規 | `src/hooks/useReviewPanel.ts` | ReviewPanel用の状態管理hook。diffBase/diffMode/selectedFile等のUI状態を管理 |
| 新規 | `src/hooks/useDiffFileTree.ts` | ファイル一覧取得→Rustツリー構築コマンド呼び出しの統合hook |
| 変更 | `src/screens/MainLayout.tsx` | 右パネル内のvertical Group構成を変更。上部にReviewPanel、下部にRightSidebarBottomを配置 |
| 変更 | `src/screens/useWorktreeState.tsx` | ReviewPanel関連のUI状態を管理対象に追加 |
| 変更 | `src/types/workspace-state.ts` | layoutにReviewPanel関連の永続化フィールドを追加 |

**Rustコマンド設計**:

```rust
// src-tauri/src/git/diff_tree.rs

#[derive(Serialize, Deserialize)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Serialize)]
pub struct DiffTreeNode {
    pub name: String,           // 表示名（結合済みパス例: "src/components"）
    pub path: String,           // フルパス
    pub node_type: String,      // "file" | "folder"
    pub status: Option<String>, // ファイルの変更ステータス
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNode>,
}

// フラットなファイル一覧 → ツリー構造に変換
// 単一子ディレクトリの自動結合を実行
pub fn build_tree(entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNode>
```

```rust
// Tauriコマンド（commands.rs）
#[tauri::command]
pub async fn build_diff_file_tree(
    entries: Vec<DiffFileEntry>,
) -> Result<Vec<DiffTreeNode>, GitError>
```

**レイアウト構成**:

```text
右パネル (Panel "right")
├─ RightPanelHeader
└─ Group(vertical)
   ├─ Panel "review" (collapsible)
   │  └─ ReviewPanel
   │     └─ Group(horizontal)
   │        ├─ Panel "diff-files" (ファイルツリー)
   │        └─ Panel "diff-view" (差分表示)
   │           ├─ DiffToolbar
   │           └─ DiffViewerSection
   └─ Panel "right-bottom" (collapsible)
      └─ RightSidebarBottom (Terminal/Comments)
```

**データフロー**:

```text
[branch-base基準]
useBranchDiffFiles → ChangedFile[] → invoke("build_diff_file_tree") → DiffTreeNode[] → DiffFileTree描画

[head基準]
useGitStatus → stagedFiles[] + changedFiles[] → invoke("build_diff_file_tree") → DiffTreeNode[] → DiffFileTree描画
```

**流用する既存コード**:
- `DiffToolbar` — diffモード切り替え・hunkナビゲーション
- `DiffViewerSection` — 画像/マークダウン/テキストの条件分岐表示
- `ImageDiffViewer`, `MarkdownDiffViewer` — 画像・マークダウン差分表示
- `StatusIcon`, `statusColor` — ツリー内のステータスアイコン表示に流用
- `useHunks` — Rust IPC (`compute_diff_hunks`) によるdiff hunks計算・ナビゲーション
- `useDiffOperations` — Rust IPC (`compute_diff_hunks`, `generate_group_patch`) によるグループ単位stage/unstage
- `useBranchDiffFiles` — branch-base基準のファイル一覧取得
- `useGitStatus` — staged基準のファイル一覧取得（stagedFiles/changedFiles）
- `useImageDiff` — 画像diff取得
- Rustコマンド — `get_branch_diff_summary`, `get_file_at_branch_base`, `get_staged_content`, `git_stage`, `git_unstage`, `compute_diff_hunks`, `generate_group_patch`等（変更なし）

**影響するテスト**:
- `src-tauri/src/git/diff_tree.rs` (Rust): ツリー構築・単一子ディレクトリ結合・空入力・深いネスト
- `DiffFileTree.test.tsx`: ツリー描画・折りたたみUI・ファイル選択
- `useReviewPanel.test.ts`: diffBase切り替え・diffMode切り替え・selectedFile管理
- `useDiffFileTree.test.ts`: ファイル一覧からツリー構築コマンド呼び出しの統合テスト
- 既存テスト（`useHunks`, `computeHunks`, `FileStatusItem`等）: 変更なし
