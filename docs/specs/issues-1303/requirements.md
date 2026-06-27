# Requirements

## Type

リファクタリング / アーキテクチャ移行（milestone [12] クリーンアーキテクチャ移行）。

repository status の **staged / changed 分類ロジックを frontend から Rust-owned read model へ移し**、production 未消費の status 系 frontend dead code を削除する。ReviewPanel の外部から観測可能な振る舞いは不変に保つ。

関連: #1303（本 ISSUE） / Blocks: #878 final dead-code sweep / 親方針: `.claude/rules/rust-first-logic.md`

## 背景と目的

CLAUDE.md / `rust-first-logic.md` の大方針「全てのアプリケーションロジックは Rust に置く。frontend はインターフェースに徹する」に従い、現在 frontend に残っている repository status 関連の domain decision を整理する。

コード調査の結果、frontend の status ロジックは次の 2 群に分かれることが判明した。

1. **production で生きているロジック** = `useReviewSnapshot` の staged / changed split（`ReviewPanel` が消費）。
2. **production 未消費（テストのみ）の dead code** = `useGitStatus` 全体（`statusMap` 構築・`toFileStatus`・staged / changed split を含むフック全体）、および `applyStatusToTree`（file tree への status overlay 伝播・ディレクトリ集約）。

本変更の目的は、(1) を Rust read model へ移して frontend を描画専従にし、(2) を dead code として削除することで、#878（final dead-code sweep）が前提とする「frontend に repository status の分類規則が残っていない」状態を作ることである。

### 現状のコード調査（事実）

#### ① `useReviewSnapshot`（`src/hooks/useReviewSnapshot.ts`）の staged / changed split — 変更前の生きたロジック

- staged / changed split — 変更前は `visibleSnapshot.status.filter(entry => entry.index_status !== "none")` を staged、`filter(entry => entry.worktree_status !== "none" && entry.worktree_status !== "ignored")` を changed と判定する domain decision が frontend にあった。backend command `get_review_snapshot` が返す `ReviewSnapshot.status`（生の git status 全件、ignored 除外済み）を frontend で再分類していた。
- `ReviewPanel.tsx`（行 179-180 で `stagedFiles` / `changedFiles` を受け取り、行 222-223, 448, 466 で消費）が staged/changes セクション分類・diff path 一覧・選択判定（`determineSectionForFile`）・stage all / unstage all の対象算出に使う。

#### ② `useReviewSnapshot` が消費する backend read model の変更前状態

- Tauri command `get_review_snapshot`（`src-tauri/src/adaptor/controller/command/code/review.rs`）は `ReviewSnapshotDto`（`src-tauri/src/usecase/code_dto.rs:122-136`）を返す。
- 変更前の `ReviewSnapshotDto` は `version` / `stale` / `loading` / `limited` / `base` / `files` / `status: Vec<FileStatusDto>` / `diff_stats` / `tree` / `staged_tree` / `changes_tree` / `staged_file_count` / `changes_file_count` を持っていた。
- backend は `head_review_snapshot`（`review_usecase.rs:858-906`）で staged/changed の **件数**（`staged_file_count` / `changes_file_count`）を `index_status != "none"` / `worktree_status != "none" && != "ignored"` の規則で算出済みだった。**しかし変更前は staged/changed の集合（path リスト）を frontend へ渡しておらず**、`status` 全件と count のみを渡していた。frontend(`useReviewSnapshot`) がその count と同じ規則で集合を再構成していた点が「frontend に残った split decision」である。

#### ③ `useGitStatus`（`src/hooks/useGitStatus.ts`） — production 未消費の dead code

- `useGitStatus` を import している production コードは存在しない（参照は `useGitStatus.test.ts` と `ReviewPanel.test.tsx` の不要な `vi.mock` 宣言のみ。`ReviewPanel` 本体は `useGitStatus` を import していない）。フック全体が dead code。
- フック内には `toFileStatus(entry)`（`index_status` / `worktree_status` を表示用 `FileStatus` へ分類する優先順位ルール）、`statusMap`（`<rootPath>/<entry.path>` をキーに `FileStatus` を引く Map）の構築、および staged / changed split が含まれるが、いずれも production からの消費者を持たない。

#### ④ `applyStatusToTree`（`src/lib/applyStatusToTree.ts`） — production 未消費の dead code

- `computeFoldersWithChanges` / `applyStatusRecursive` — file tree node への status 付与、ディレクトリの変更有無集約（フォルダ配下に変更があれば folder を `modified` 表示にする）、ignored の親→子伝播。
- import している production コードは存在しない（`applyStatusToTree.test.ts` のみ）。file tree への status overlay は production に配線されていない。

## 確定した方針（合意済み）

- 当初の requirements は「生きた split は `useGitStatus` にある」としていたが、実コード調査の結果これは誤りで、生きた split は `useReviewSnapshot` にあり `useGitStatus` 自体が production 未消費の dead code であることが確認された。本 requirements はこの実コードに合わせて修正済みである。
- dead code（`useGitStatus` フック全体とそのテスト、`applyStatusToTree` とそのテスト、`ReviewPanel.test.tsx` の `useGitStatus` mock）は **production 未消費の dead code として丸ごと削除する**。Rust 側に同等の status 集約（directory aggregation / status propagation / unknown fallback / 表示用 `FileStatus` 分類）read model を新設しない。
- 生きているロジックである **`useReviewSnapshot` の staged / changed split を Rust read model へ移す**。backend が `ReviewSnapshotDto` で staged / changed の振り分け結果（集合）のみを公開し、`status` 全件フィールドは transport に残さない。`useReviewSnapshot` はそれを保持・公開するだけにする。ReviewPanel の観測可能な振る舞いは不変とする。

> 補足（人間レビュー向け）: ISSUE #1303 の完了条件は「Rust test で file status propagation / directory aggregation / unknown status fallback を検証」を挙げているが、これらは overlay 系（`applyStatusToTree`）に対応する。本 ISSUE では overlay 系を **移行ではなく削除** するため、当該 Rust test 群は対象外となる。Rust test の対象は staged / unstaged split に限定する。この差分は合意済み。

## スコープ

- **①** staged / changed split の判定を Rust read model 側へ移す。`get_review_snapshot` が返す `ReviewSnapshotDto` は `status` 全件フィールドを持たず、staged / changed の振り分け結果（集合）を UI-ready に持つ。
- **②** `useReviewSnapshot` から staged / changed split の decision を除去し、backend が返した read model の集合をそのまま保持・公開する形にする。
- **③** `useGitStatus`（`src/hooks/useGitStatus.ts`）とそのテスト（`useGitStatus.test.ts`）を削除する。これにより `statusMap` 構築・`toFileStatus`・useGitStatus 内の staged / changed split が同時に消える。
- **④** `applyStatusToTree`（`src/lib/applyStatusToTree.ts`）とそのテスト（`applyStatusToTree.test.ts`）を削除する。
- **⑤** `ReviewPanel.test.tsx` の不要な `useGitStatus` mock を削除する。frontend test を「invoke 結果（staged/changed）の描画 / loading / error」に寄せ、削除した分類規則の単体テストを除去する。staged/unstaged split の規則テストは Rust 側に置く。

## 非スコープ

- git host provider integration（#985）。
- review comment persistence（#1132）。
- file tree の visual redesign、および file tree への status overlay 表示の新設（overlay 系は削除であり、新規 UI は作らない）。
- review snapshot の取得経路（git2 walk・version 採番・stale/loading/limited フラグの意味・`base` 別の snapshot 構築方針）そのものの変更。本変更は取得済み status の staged/changed 分類の read model 化に閉じる。
- `get_review_snapshot` 以外の repository state command（`get_git_status_snapshot` / diff stats / branch cards / diff file tree 等）の read model 変更。`get_git_status_snapshot` は本変更の対象外（`useGitStatus` 経由でのみ参照され、その `useGitStatus` を削除するため）。
- ReviewPanel の機能・レイアウト・diff base 選択ロジックの変更（staged/changed の供給元が backend read model の集合に変わる配線変更を除く）。
- 表示用 `FileStatus`（`modified` / `added` / `deleted` / `untracked` / `ignored` / `null`、`src/types/file-tree.ts`）分類の Rust 移植（これは overlay 系専用で削除対象のため移植しない）。

## 要求事項

- staged / changed の振り分け規則が frontend に存在せず、Rust read model（`ReviewSnapshotDto`）が決定すること（①）。
- `useReviewSnapshot` が backend read model の staged / changed 結果（集合）を取得して保持・公開するだけになり、split の decision（`status` からの filter 再計算）を含まないこと（②）。
- `useGitStatus` とそのテストが削除されていること（③）。
- `applyStatusToTree` とそのテストが削除されていること（④）。
- ReviewPanel が依存する staged / changed 情報が backend read model 由来の値として従来と同一内容で供給され、ReviewPanel の観測可能な振る舞い（staged/changes セクションの分類、diff path 一覧、選択判定、stage all / unstage all 対象算出）が不変であること。
- read model が backend-owned であり、Tauri 以外の将来 client surface からも同じ shape を読める形であること（full-retention / frontend 再計算経路を増やさない）。
- staged / unstaged split の規則を担保する Rust test が存在すること。
- frontend test が invoke 結果（staged/changed）の描画・loading・error に寄ること。

## 受け入れ基準の概要

- frontend grep で staged/changed split の判定（`index_status` / `worktree_status` の `!== "none"` による振り分け）が `useReviewSnapshot` に残っていないことを確認できる。
- `useGitStatus`（`statusMap` 構築・`toFileStatus` を含む）と `applyStatusToTree` およびそれらのテストファイルが存在しないことを確認できる。
- `ReviewPanel.test.tsx` に `useGitStatus` の mock が残っていないことを確認できる。
- Rust test で staged / unstaged split が検証されている。
- ReviewPanel の既存テスト（staged/changes 分類・diff path 供給）が、backend read model 由来の値で従来どおり通ること。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- backend が返す staged / changed read model の transport 形（`ReviewSnapshotDto` に staged / changed の集合フィールドを持たせ、`status` 全件フィールドは持たせない）の具体は design.md で決定する。本 requirements では「staged/changed の振り分けを Rust が所有し frontend は描画のみ」という性質のみを要求とする。
- ReviewPanel が必要とするのは staged / changed の `GitFileStatus` 集合（path を含む）であり、これらは backend read model から従来と同一内容で供給できる前提とする。
- 現行 staged / changed split の規則（`index_status !== "none"` → staged、`worktree_status !== "none" && !== "ignored"` → changed）を Rust 側で等価に再現し、観測可能な結果を変えない。backend は既に同じ規則で `staged_file_count` / `changes_file_count` を算出しているため、集合フィールドはその規則の延長で構築できる。
- version / stale / loading / limited フラグの意味と採番、および `useReviewSnapshot` の version dedup・race 制御は現状を維持し、本変更で変更しない。
- overlay 系削除に伴い `FileStatus`（`src/types/file-tree.ts`）型自体は `FileNode.status` フィールドが file tree で参照しているため削除しない。型の要否整理は #878 final dead-code sweep 側の判断に委ね、本 ISSUE では未使用化した分類ロジックの削除に閉じる。

## Open Questions

なし。
</content>
