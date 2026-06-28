# Requirements

## Type

リファクタリング / アーキテクチャ移行（milestone [12] クリーンアーキテクチャ移行）。

frontend が branch name / worktree directory / worktree path / shell escaping を導出している domain rule を Rust-owned usecase / query command / read model の背後へ移し、frontend を「ユーザー入力の受付」と「Rust が返した候補値の表示」に縮小する。

関連: #1302（本 ISSUE） / Depends on: #986（Notion branch derivation 境界）, #1133（workspace path / app glue / command 登録の Rust 所有境界） / Blocks: #878 final dead-code sweep / 親方針: `.claude/rules/rust-first-logic.md`

## 背景と目的

CLAUDE.md / `rust-first-logic.md` の大方針「全てのアプリケーションロジックは Rust に置く。frontend はインターフェースに徹する」に従い、現在 frontend に残っている branch / path 導出の domain decision を Rust へ移す。

frontend は issue 選択や input field の一時値といったユーザー入力を Rust へ渡し、Rust から返った branch 候補・worktree path 候補・escaping 済み path を表示・利用するだけにする。これにより #878（final dead-code sweep）が前提とする「frontend に branch/worktree path の domain rule が残っていない」状態を作る。

### 現状のコード調査（事実）

#### ① `generateIssueBranchName`（`src/lib/issueBranch.ts`） — production で生きている

- `feat/issues/${issueNumber}` を生成する規則。
- production 消費者: `CreateWorktreeModal.tsx`（行 326 `toggleBranch`、行 540 既存 worktree 除外フィルタ、行 718 ラベル/選択）。issue number → default branch name の domain rule が frontend にある。

#### ② `generateNotionBranchName`（`src/lib/notionBranch.ts`） — production 未消費（テストのみ）

- Notion の branch name property を sanitize（空白→`-`、許可外文字除去、連続 `-` 圧縮、先頭末尾 `-`/`/` 除去）し、空なら `pageId` 由来 `notion/<short8>`、それも無ければ `notion-task` を返す fallback 規則。任意の `prefix` 前置にも対応。
- production 消費者は存在しない（`notionBranch.test.ts` のみが import）。frontend に Notion branch derivation は配線されていない dead/test-only コードだが、Notion property fallback を含む規則そのものは #986 が確定する Notion branch derivation 境界（Rust）に属するべき domain rule である。

#### ③ `computeWorktreeDir` / `branchToDir`（`src/lib/worktreePath.ts`） — production で生きている

- `computeWorktreeDir(repoPath)`: repo の親ディレクトリ + `<repoName>-worktrees` を組み立てる規則。
- `branchToDir(branch)`: branch name の `/` を `-` に置換し worktree directory name にする規則。
- production 消費者: `CreateWorktreeModal.tsx` `handleCreate`（行 182・190）。frontend が `${worktreeDir}/${dirName}` で worktree path 文字列を組み立て、`create_worktree` command に `worktreePath` として渡している（行 191-198）。worktree directory / path の導出規則が frontend にある。

#### ④ `normalizePath`（`src/lib/normalizePath.ts`） — production で広く使用

- `p.replace(/\\+/g, "/")`（backslash → forward slash 正規化）。
- production 消費者: `useHandleOpenFile.ts`、`useWorkspaceNavigation.ts`、`useWorktreeList.ts`、`lib/agentStateUtils.ts`。いずれも **backend が返した path / event payload の `worktree_id` / `worktree_path` を比較キーとして突き合わせる**ために使用しており、新しい path を導出する規則ではなく、backend-owned path の比較キー正規化である。
- **方針確定（合意済み）**: backend が event / read model で正規化済み（canonical, forward-slash）の path を返すよう Rust 側で吸収し、frontend の `normalizePath` 呼び出しを全廃する（後述 ⑥）。

#### ⑤ `quotePathForShell` / `quotePathsForShell`（`src/lib/quotePathForShell.ts`） — production で生きている（実行用途）

- shell metacharacter を含む path を single-quote escaping する規則と、複数 path を空白結合する規則。
- production 消費者: `TerminalPanel.tsx`。file drop（行 101 複数 / 行 141 単一）時に **escaping 済み文字列を `writeToTerminal` で PTY へ書き込む**。これは「frontend が shell command 用文字列を組み立てて実行経路へ渡す」アンチパターンに該当し、表示専用ではなく実行用途である。

## スコープ

- **①** issue number → branch name 導出（`generateIssueBranchName` 相当）を Rust の usecase / query command へ移す。`CreateWorktreeModal` は Rust から返った branch 候補を表示・選択に使うだけにする。
- **②** Notion property → branch name 導出（`generateNotionBranchName` 相当、sanitize + pageId fallback + `notion-task` fallback + prefix 規則）を #986 が確定する Rust の Notion branch derivation 境界へ移し、Rust test で検証する。frontend の `notionBranch.ts` とそのテストは削除する。
- **③** worktree directory / worktree path 導出（`computeWorktreeDir` / `branchToDir` および `${dir}/${branchToDir}` 結合）を Rust へ移す。`CreateWorktreeModal.handleCreate` は worktree path を frontend で組み立てず、Rust に repo path と branch を渡して導出させる（`create_worktree` 経路で Rust が worktree path を所有する形にする。具体的な command 形は design.md で確定）。
- **④** shell escaping（`quotePathForShell` / `quotePathsForShell`）を Rust 所有にする。`TerminalPanel` の file drop は frontend で shell 文字列を組み立てず、Rust に path を渡して argv / escaping 済み文字列を得るか、Rust 側で PTY へ書き込む（具体方式は design.md で確定）。表示専用に残す部分があれば、実行用ではないことがわかる名前と test にする。
- **⑤** 上記移行に伴い `src/lib/issueBranch.ts` / `src/lib/notionBranch.ts` / `src/lib/worktreePath.ts` / `src/lib/quotePathForShell.ts` を削除するか、invoke wrapper / UI-only helper に縮小する。対応するフロントエンドの分類規則 unit test は削除し、規則の test は Rust 側に置く。
- **⑥** path 正規化（`normalizePath`、backslash→forward-slash）を backend へ吸収する。backend が event / read model で正規化済み（canonical）path を返すよう Rust 側で保証し、frontend の `normalizePath` 呼び出し（`useHandleOpenFile` / `useWorkspaceNavigation` / `useWorktreeList` / `agentStateUtils`）を全廃する。`src/lib/normalizePath.ts` とそのテストは削除する。比較キーの突き合わせ結果（worktree_id / worktree_path のマッチング）は不変に保つ。

## 非スコープ

- GitHub/GitLab など git host integration の migration（#985）。
- Notion integration module 本体の migration（#986）。本 ISSUE は #986 が確定する Notion branch derivation 境界に branch name 規則を載せることに閉じ、Notion fetch / property 取得経路そのものは変更しない。
- visual redesign / `CreateWorktreeModal` の機能・レイアウト変更（branch/path 供給元が Rust に変わる配線変更を除く）。
- `create_worktree` の worktree 作成ロジック（git2 worktree add・branch 作成）そのものの変更。本 ISSUE は worktree path の **導出** を Rust 所有にすることに閉じ、作成手順は変更しない。
- terminal / PTY の入出力経路そのものの再設計（file drop 時の escaping 所有を Rust に移すことを除く）。

## 要求事項

- issue number → branch name の導出規則が frontend に存在せず、Rust（usecase / query command）が決定し、`CreateWorktreeModal` は返却された候補値を表示・利用するだけであること（①）。
- Notion property → branch name の導出規則（sanitize・pageId fallback・`notion-task` fallback・prefix 規則）が Rust に存在し、frontend の `notionBranch.ts` とそのテストが削除されていること（②）。
- worktree directory / worktree path の導出規則が frontend に存在せず、Rust が決定すること。`CreateWorktreeModal.handleCreate` が worktree path 文字列を frontend で組み立てないこと（③）。
- shell escaping が frontend の実行経路に存在せず、`TerminalPanel` の file drop が frontend で shell command 文字列を組み立てないこと。`quotePathForShell` が表示専用として残る場合は実行用でないことが名前と test で明確であること（④⑤）。
- `src/lib/issueBranch.ts` / `src/lib/notionBranch.ts` / `src/lib/worktreePath.ts` が削除されるか、invoke wrapper / UI-only helper に縮小されていること。`quotePathForShell` が削除されるか表示専用 helper として明確化されていること（⑤）。
- backend が正規化済み path を返し、frontend の `normalizePath` 呼び出しと `src/lib/normalizePath.ts` が全廃されていること。worktree_id / worktree_path の突き合わせ結果が不変であること（⑥）。
- 移行先の Rust が backend-owned read model / command であり、Tauri 以外の将来 client surface からも同じ規則・shape を利用できること（full-retention / frontend 再計算経路を増やさない）。
- branch / path derivation の正常系・空白・記号・重複回避・Notion property fallback が Rust test で検証されていること。
- frontend の観測可能な振る舞い（issue からの worktree 作成、既存 branch 除外、file drop 時の terminal 入力）が従来と不変であること。

## 受け入れ基準の概要

- frontend grep で branch / worktree path の導出規則（`feat/issues/` テンプレート、`<repoName>-worktrees` 組み立て、`branch` の `/`→`-` 置換、`${dir}/${name}` 結合、Notion sanitize/fallback）および `normalizePath` 呼び出しが src 配下に残っていないことを確認できる。
- `src/lib/issueBranch.ts` / `src/lib/notionBranch.ts` / `src/lib/worktreePath.ts` とそれらのテストが削除されているか、invoke wrapper / UI-only helper に縮小されていることを確認できる。
- `quotePathForShell` が削除されているか、表示専用と分かる名前・test に整理されていることを確認できる。
- `TerminalPanel` の file drop 経路で frontend が shell 文字列を組み立てていないことを確認できる。
- Rust test で issue branch 導出・Notion branch 導出（property fallback 含む）・worktree directory/path 導出・shell escaping の正常系/空白/記号/重複回避が検証されている。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 移行先の Rust command / read model の具体的な shape（既存 `create_worktree` command を拡張して worktree path を Rust が導出するか、別途 branch/path 候補を返す query command を新設するか等）は design.md で確定する。本 requirements では「導出を Rust が所有し frontend は表示・利用のみ」という性質のみを要求とする。
- `generateNotionBranchName` は現状 production 未配線（テストのみ）だが、規則は #986 が確定する Notion branch derivation 境界（Rust）に属するべき domain rule とみなし、frontend ファイル/テストを削除して規則を Rust 側に Rust test 付きで担保する。frontend に新規 Notion branch UI は作らない。
- `branchToDir`（`/`→`-`）と既存 worktree branch の重複回避（`CreateWorktreeModal` の `worktreeBranchNames` / `existingNames` による除外）の観測結果は不変に保つ。重複回避判定に必要な既存 branch 一覧は引き続き backend の worktree/branch read model から供給する。
- `TerminalPanel` の escaping 移行後も、file drop 時に terminal へ入力される文字列の内容（quote の有無・形）は現行 `quotePathForShell` と等価に保つ。
- shell escaping の現行実装は POSIX shell（single-quote）前提であり、対象シェルの差異（Windows 等）は本 ISSUE では現行と同等に保ち拡張しない。
- ⑥ の canonical 化は forward-slash 正規化（現行 `normalizePath` と等価）を backend の path 出力に保証することを指し、path の意味（指す対象）は変えない。どの emit / read model で正規化を保証するかの具体箇所は design.md で確定する。

## Open Questions

なし。
