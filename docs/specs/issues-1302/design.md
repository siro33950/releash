# Design

本書は #1302（branch / worktree path / shell escaping / path 正規化の導出規則を Rust 所有化する）の実装設計を確定する。`requirements.md` と `behavior.md` を入力とする。本 ISSUE はリファクタリング / アーキテクチャ移行であり、観測可能な振る舞い（生成 branch 名・作成 worktree path・terminal 入力文字列・path 突き合わせ結果）は不変に保つ。

## 概要

frontend に残る 5 つの導出規則（① issue→branch、② Notion property→branch、③ worktree directory/path、④ shell escaping、⑥ path 正規化）を、Rust の domain rule + usecase / query command / read model の背後へ移す。frontend は「ユーザー入力の受付」と「Rust が返した候補値・正規化済み値の表示・利用」に縮小する。

各規則の移行先:

| # | 現状 (frontend) | 移行先 (Rust) | frontend 側結果 |
|---|---|---|---|
| ① | `lib/issueBranch.ts` `generateIssueBranchName` | `IssueInfoDto.default_branch_name`（read model に導出値を載せる） | ファイル削除、`issue.default_branch_name` を参照 |
| ② | `lib/notionBranch.ts` `generateNotionBranchName` | `domain/notion` の branch 導出 rule + Rust test | ファイル・テスト削除（新規 UI なし） |
| ③ | `lib/worktreePath.ts` `computeWorktreeDir`/`branchToDir` | `domain/repository` の worktree path 導出 rule、`create_worktree` 内部で導出 | ファイル削除、`worktree_path` 引数を渡さない |
| ④ | `lib/quotePathForShell.ts` | `domain` の shell escaping rule + 新規 `write_paths_to_pty` command | ファイル削除、raw path を渡すだけ |
| ⑥ | `lib/normalizePath.ts` | backend の read model / event 出力を forward-slash canonical に保証 | ファイル・テスト削除、`normalizePath` 呼び出し全廃 |

## 変更対象

### Rust (src-tauri/src/)

- `domain/repository/` — worktree directory / path 導出 rule（新規 value object もしくは関数）と test。
- `domain/notion/` — Notion property → branch name 導出 rule（sanitize + pageId fallback + `notion-task` fallback + prefix）と test。
- `domain/git_host/`（または `usecase/git_host/dto.rs`）— issue number → branch name 導出 rule。
- `usecase/git_host/dto.rs` — `IssueInfoDto` に `default_branch_name` フィールド追加、`From<IssueInfo>` で導出値を充填。
- `usecase/repository_usecase.rs` — `create_worktree` が repo_path + branch から worktree path を導出して gateway へ渡す。
- `adaptor/controller/command/repository/worktree.rs` — `create_worktree` command の引数から `worktree_path` を撤去。
- `adaptor/controller/command/pty_session/commands.rs` — `write_paths_to_pty` command 新設。
- `usecase/pty_session/io_usecase.rs` — paths → escaping 済み文字列導出 + write を行う関数追加。
- `adaptor/gateway/repository/worktree.rs` / `branch_card.rs` / `watch.rs` — 返却 path / emit path を forward-slash canonical に正規化。
- `adaptor/gateway/repository/`（WorkspaceStatus emit 箇所）— `worktree_id` / `worktree_path` を canonical 化。

### Frontend (src/)

- 削除: `lib/issueBranch.ts`、`lib/notionBranch.ts`(+test)、`lib/worktreePath.ts`(+test)、`lib/quotePathForShell.ts`(+test)、`lib/normalizePath.ts`(+test)。
- `components/workspace/CreateWorktreeModal.tsx` — `generateIssueBranchName` → `issue.default_branch_name`、`computeWorktreeDir`/`branchToDir` 撤去、`create_worktree` invoke から `worktreePath` 撤去。
- `components/panels/TerminalPanel.tsx` — file drop が raw path 配列を `write_paths_to_pty` へ渡す。`quotePathForShell`/`writeToTerminal(escaped)` を撤去。
- `hooks/useHandleOpenFile.ts` / `useWorkspaceNavigation.ts` / `useWorktreeList.ts`、`lib/agentStateUtils.ts` — `normalizePath` 呼び出し撤去（backend が正規化済み値を返すため直接比較）。
- `types/git.ts` — `IssueInfo` に `default_branch_name: string` 追加。

## アーキテクチャと責務分割

clean architecture の段階移行方針（`src-tauri/AGENTS.md`）に従う。導出規則は **domain rule** として infrastructure 非依存に置き、usecase が組み立て、controller が Tauri 入出力へ変換、read model（DTO）が frontend へ shape を渡す。

### ① issue number → branch name

- domain: `feat/issues/<number>` を返す純関数 rule（配置先は git_host domain。issue という概念に紐づくため）。
- read model: `IssueInfoDto.default_branch_name` に `From<IssueInfo>` 変換時へ充填。frontend は issue 一覧取得（`get_cached_issues` / `fetch_issues`）の戻り値に既に含まれる値を表示・dedup・選択に使う。
- **per-issue の invoke を増やさない**（full-recompute 回路を作らない）。既存の issue 一覧 read model に値を載せることで、追加の往復なしに frontend が候補値を得る。

### ② Notion property → branch name

- domain: `domain/notion` に sanitize（`trim` → 連続空白→`-` → 許可外文字 `[^a-zA-Z0-9/_-]` 除去 → 連続 `-` 圧縮 → 先頭末尾 `-`/`/` 除去）+ pageId fallback（`notion/<short8>`）+ `notion-task` fallback + prefix 前置 rule を純関数として置く。現行 `notionBranch.ts` のロジックと等価。
- production 配線は作らない（frontend に Notion branch UI なし）。Rust test のみで規則を担保する。
- 既存 `NotionTask.branch_name`（API 取得済み値）との関係は本 ISSUE のスコープ外。本 ISSUE は frontend の dead/test-only な導出規則を Rust domain rule + test として移植することに閉じる。

### ③ worktree directory / path

- domain: `worktree_dir(repo_path) -> String`（repo 親ディレクトリ + `<repoName>-worktrees`）と `branch_to_dir(branch) -> String`（`/`→`-`）、両者を結合する `worktree_path(repo_path, branch)` を純関数 rule として `domain/repository` に置く。現行 `worktreePath.ts` と等価。
- usecase: `RepositoryUsecase::create_worktree` が `worktree_path(repo_path, branch)` を導出し gateway へ渡す。
- controller: `create_worktree` command の引数から `worktree_path` を撤去（`repo_path`, `branch`, `create_branch`, `base_branch` のみ受ける）。worktree 作成手順（git2 worktree add）自体は不変。
- frontend: `handleCreate` は worktree path を組み立てず、`create_worktree` に repo path と branch を渡すだけ。

### ④ shell escaping

- domain: `quote_path_for_shell(path) -> String`（POSIX single-quote escaping）と `join_quoted_paths(paths) -> String`（escaping 後に空白結合）を純関数 rule として置く。現行 `quotePathForShell.ts` と文字単位で等価（メタ文字集合・`'` のエスケープ `'\''`・空白 join を保つ）。
- command: `write_paths_to_pty(pty_id, paths: Vec<String>)` を新設。Rust が escaping+結合して PTY へ書き込む（既存 `write_pty` の write 経路を再利用）。
- frontend: file drop（単一 / 複数）は raw path を `Vec<String>`（単一は要素 1）にして `write_paths_to_pty` を呼ぶ。shell 文字列の組み立ては行わない。

### ⑥ path 正規化（canonical 化）

- backend が read model / event で返す path-bearing フィールドを forward-slash canonical（現行 `normalizePath` と等価、`\`→`/`）に保証する。対象は frontend が突き合わせキーに使う値:
  - `WorktreeEntryDto.path`（`create_worktree` 戻り値）
  - `WorkspaceStatus.worktree_id` / `worktree_path`（`list_workspace_statuses` 戻り値・`workspace-status-changed` event payload）
  - `BranchCardDto.worktree_path`
  - file open / file-change 経路で frontend が比較キーに使う path
- 既に `watch.rs` に `canonicalize_event_path` と `p.replace('\\', "/")` が存在する。これと等価な forward-slash 正規化を共有 helper として整理し、上記 emit / DTO 生成箇所で適用する。
- frontend の `normalizePath` 呼び出しは全廃。比較は backend が正規化済みの値同士の直接等価比較になる。突き合わせ結果（worktree_id / worktree_path のマッチング）は不変。

## データモデルまたは型

### Rust

```rust
// domain/git_host (issue branch rule)
pub fn issue_branch_name(number: u64) -> String { format!("feat/issues/{number}") }

// usecase/git_host/dto.rs
pub struct IssueInfoDto {
    // 既存フィールド...
    pub default_branch_name: String, // 追加: From<IssueInfo> で issue_branch_name(number) を充填
}

// domain/repository (worktree path rule)
pub fn worktree_dir(repo_path: &str) -> String;       // <parent>/<repoName>-worktrees
pub fn branch_to_dir(branch: &str) -> String;         // '/' -> '-'
pub fn worktree_path(repo_path: &str, branch: &str) -> String; // dir + '/' + branch_to_dir

// domain/notion (notion branch rule)
pub fn notion_branch_name(branch_name_property: &str, page_id: Option<&str>, prefix: Option<&str>) -> String;

// domain (shell escaping rule)
pub fn quote_path_for_shell(path: &str) -> String;
pub fn join_quoted_paths(paths: &[String]) -> String;

// 共有 path 正規化 helper
pub fn to_canonical_forward_slash(path: &str) -> String; // '\' -> '/'
```

```rust
// create_worktree command（worktree_path 撤去後）
#[tauri::command]
pub async fn create_worktree(
    state: State<'_, AppState>,
    repo_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<WorktreeEntryDto, AppError>

// 新規 command
#[tauri::command]
pub fn write_paths_to_pty(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
    paths: Vec<String>,
) -> Result<(), String>
```

### Frontend

```ts
// types/git.ts
export interface IssueInfo {
  // 既存フィールド...
  default_branch_name: string; // 追加
}
```

## 処理フロー

### issue から worktree 作成（①③⑥）

1. frontend が `get_cached_issues` / `fetch_issues` を呼ぶ → Rust が `default_branch_name` 入りの `IssueInfoDto[]` を返す。
2. `CreateWorktreeModal` は `issue.default_branch_name` を表示・選択・既存 worktree 除外 dedup に使う（dedup 対象の既存 branch 一覧は従来通り backend の branch read model から供給）。
3. 作成実行で frontend は `create_worktree({ repoPath, branch, createBranch, baseBranch })` を invoke（`worktreePath` を渡さない）。
4. Rust usecase が `worktree_path(repo_path, branch)` を導出し gateway へ渡して worktree を作成。`WorktreeEntryDto.path` は canonical（forward-slash）で返る。
5. frontend は戻り値・後続 event を `normalizePath` なしで直接利用・突き合わせる。

### terminal file drop（④）

1. `native-file-drop` / drag-drop で raw path（単一 or 複数）を得る。
2. frontend が `write_paths_to_pty({ ptyId, paths })` を invoke。
3. Rust が各 path を `quote_path_for_shell` し、複数なら空白結合して PTY へ書き込む。terminal へ入る文字列は現行と等価。

### Notion branch rule（②）

- frontend 配線なし。`domain/notion` の `notion_branch_name` を Rust test で検証する。`notionBranch.ts` とテストは削除。

## エラー処理

- 各 domain rule は純関数（失敗しない / fallback 内包）。空入力・記号のみ等は現行同様 fallback（②は `notion-task`、その他は空文字や元値）に倒す。
- `create_worktree` の worktree 作成失敗は既存の `RepositoryError` → `UsecaseError` → `AppError` 経路を維持（変更しない）。
- `write_paths_to_pty` は既存 `write_pty` と同じく `Result<(), String>`。pty_id 不在等は既存 PTY write のエラー処理に倣う。空配列は no-op（書き込みなし）。
- ⑥ の正規化は冪等な文字列変換であり失敗しない。

## テスト方針

### Rust（規則の test は Rust 側に置く）

- `issue_branch_name`: 正常系（`feat/issues/1302`）。
- `notion_branch_name`: 正常系・空白圧縮・許可外記号除去・連続 `-` 圧縮・先頭末尾除去・空→pageId fallback（`notion/<short8>`）・pageId なし→`notion-task`・prefix 前置（現行 `notionBranch.test.ts` のケースを移植）。
- `worktree_dir` / `branch_to_dir` / `worktree_path`: 正常系・`/`→`-` 置換・親ディレクトリ組み立て（現行 `worktreePath.test.ts` を移植）。
- `quote_path_for_shell` / `join_quoted_paths`: 通常 path（quote なし）・空白/記号 path（single-quote）・`'` を含む path・複数 path 空白結合（現行 `quotePathForShell.test.ts` を移植）。重複回避・空白・記号の正常系を網羅。
- `to_canonical_forward_slash`: `\`→`/` 変換と冪等性。
- `create_worktree` usecase: worktree path が repo_path+branch から導出されること（正常系）と既存のエラー系。

### Frontend

- 削除した lib のテストは削除。
- `CreateWorktreeModal` / `TerminalPanel` の振る舞いテストは、invoke 引数 shape（`create_worktree` に `worktreePath` を渡さない / `write_paths_to_pty` に raw paths を渡す）を検証する形へ更新。`@tauri-apps/api` は `vi.mock`。
- 既存の観測可能な振る舞い（issue 選択での候補表示・既存 branch 除外・file drop 入力）が不変であることを確認。

### 受け入れ確認

- frontend grep で `feat/issues/`・`<repoName>-worktrees`・`/`→`-` 置換・`${dir}/${name}` 結合・Notion sanitize/fallback・`normalizePath` 呼び出しが src 配下に残らない。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## リスクと代替案

- **③ command 署名変更のリスク**: `create_worktree` から `worktree_path` を撤去すると command の互換性が変わる。本リポジトリ内の唯一の呼び出し元は `CreateWorktreeModal` であり同時に修正するため影響は閉じる。
  - 代替案: `worktree_path` を撤去せず、別途 `derive_worktree_path(repo_path, branch)` query command を新設し frontend が導出値を取得して `create_worktree` に渡す。導出規則は Rust 所有になるが往復が 1 回増え、frontend が path を中継する形が残る。requirements の「`create_worktree` 経路で Rust が worktree path を所有」に照らし、**署名変更（Rust 内部導出）を採用**する。
- **⑥ canonical 化の網羅漏れリスク**: 正規化すべき emit / read model を取りこぼすと突き合わせが壊れる。対象は worktree_path / worktree_id を含む read model・event に限定し、列挙した全箇所に共有 helper を適用して回帰（突き合わせ結果不変）をテストで担保する。開発プラットフォーム（darwin）では `\` を含まず正規化は no-op のため既存挙動は保たれる。
- **① read model 拡張の配置**: `IssueInfoDto` は #985 で移行済みの git_host read model。導出値フィールドの追加は read model の責務内であり、git_host fetch 経路そのものは変更しない（非スコープを侵さない）。
- **④ PTY 書き込み方式**: Rust が直接 PTY へ書き込む方式を採用（escaping 済み文字列を frontend へ返して frontend が `write_pty` する方式は、frontend に「実行用文字列の中継」を残すため不採用）。

## 仮定

- ① の issue→branch 規則は既存 issue 一覧 read model（`IssueInfoDto`）に `default_branch_name` として載せる。per-issue invoke は追加しない。
- ② は production 配線を作らず、規則を `domain/notion` に Rust test 付きで移植する。`notionBranch.ts` とテストは削除する。
- ③ は `create_worktree` の引数から `worktree_path` を撤去し Rust 内部で導出する。worktree 作成手順（git2）は変更しない。
- ④ は `write_paths_to_pty` を新設し Rust が escaping + PTY 書き込みを所有する。terminal へ入る文字列は現行 `quotePathForShell` と等価。
- ⑥ は forward-slash 正規化を backend の path 出力（worktree_path / worktree_id を含む read model・event）に保証し、frontend の `normalizePath` を全廃する。突き合わせ結果は不変。
- shell escaping は POSIX single-quote 前提を維持し、シェル差異（Windows 等）は拡張しない。
- domain rule の配置（issue=git_host、worktree path=repository、notion=notion、shell escaping=共有 domain）は本書の通りとし、`src-tauri/AGENTS.md` のレイヤー規約に従う。

## Open Questions

なし。
