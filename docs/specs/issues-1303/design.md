# Design

## 概要

repository status の staged / changed 分類を、frontend (`useReviewSnapshot`) の filter による再計算から、Rust-owned read model (`ReviewSnapshotDto`) が決定する形へ移す。あわせて production 未消費の dead code（`useGitStatus` フック全体と `applyStatusToTree`、それぞれのテスト、`ReviewPanel.test.tsx` の不要 mock）を削除する。

ReviewPanel の外部から観測可能な振る舞い（staged/changes セクション分類、diff path 一覧、選択判定、stage all / unstage all 対象算出）は不変に保つ。本変更は milestone [12] クリーンアーキテクチャ移行の一環であり、`rust-first-logic.md`「全てのロジックは Rust に置く。frontend はインターフェースに徹する」に従う。

### 設計の出発点（実コード調査による前提補正）

当初 requirements は「生きた split は `useGitStatus` にある」としていたが、実コード調査の結果これは誤りで、次が確認された。requirements.md / behavior.md は本前提に合わせて修正済みである。

- `useGitStatus` を import する production コードは存在しない（参照は `useGitStatus.test.ts` と `ReviewPanel.test.tsx` の `vi.mock` 宣言のみ）。フック全体が dead code。
- ReviewPanel が実際に消費する staged / changed の生きた split は `useReviewSnapshot.ts` にあった。変更前は `get_review_snapshot` が返す `ReviewSnapshotDto.status`（生の git status 全件、ignored 除外済み）を frontend で `filter` して再構成していた。
- backend は既に `head_review_snapshot` / `branch_base_review_snapshot` で `staged_file_count` / `changes_file_count` を staged / changed と同じ規則で算出済みだったが、変更前は **集合（path リスト）を frontend へ渡していなかった**。

したがって移行は「frontend の filter 規則を backend の集合構築へ引き上げ、frontend はそれを描画専従で公開する」ことに帰着する。

## 変更対象

### backend（Rust）

| ファイル | 変更内容 |
|---|---|
| `src-tauri/src/usecase/code_dto.rs` | `ReviewSnapshotDto` から `status` 全件フィールドを除去し、公開集合として `staged_files: Vec<FileStatusDto>` / `changed_files: Vec<FileStatusDto>` を持たせる（`serde(rename_all = "camelCase")` により `stagedFiles` / `changedFiles` で transport）。 |
| `src-tauri/src/usecase/review_usecase.rs` | staged / changed split を担う共通ヘルパーを追加。`head_review_snapshot` / `branch_base_review_snapshot` の両方で `staged_files` / `changed_files` を構築して詰める。branch-base 経路では branch diff status を `FileStatusDto.worktree_status` の値域へ正規化する。`staged_file_count` / `changes_file_count` は各集合長から導出（規則・値は不変）。split 規則を検証する Rust test を追加。 |

### frontend（TypeScript）

| ファイル | 変更内容 |
|---|---|
| `src/types/review.ts` | `ReviewSnapshot` から `status` を除去し、`stagedFiles: GitFileStatus[]` / `changedFiles: GitFileStatus[]` を公開集合として持たせる。 |
| `src/hooks/useReviewSnapshot.ts` | `visibleSnapshot.status.filter(...)` 2 箇所を削除し、`visibleSnapshot.stagedFiles` / `visibleSnapshot.changedFiles` を公開。`EMPTY_SNAPSHOT` は `status` を持たず、`stagedFiles: []` / `changedFiles: []` を持つ。 |
| `src/hooks/useReviewSnapshot.test.ts` | mock snapshot を `stagedFiles` / `changedFiles` を持つ形へ更新。「backend 由来の集合をそのまま公開し再計算しない」「rootPath なし / 取得失敗で空」を検証。 |
| `src/components/panels/ReviewPanel.test.tsx` | `vi.mock("@/hooks/useGitStatus", ...)` ブロックを削除（実体未使用の不要 mock）。`useReviewSnapshot` mock の `stagedFiles` / `changedFiles` 供給はそのまま。 |

### 削除（dead code）

| ファイル | 理由 |
|---|---|
| `src/hooks/useGitStatus.ts` | production 未消費。`statusMap` 構築・`toFileStatus`・staged/changed split を内包するが消費者なし。 |
| `src/hooks/useGitStatus.test.ts` | 上記のテスト。 |
| `src/lib/applyStatusToTree.ts` | production 未消費。file tree への status overlay は配線されていない。 |
| `src/lib/applyStatusToTree.test.ts` | 上記のテスト。 |

`get_git_status_snapshot`（Tauri command）は `useGitStatus` 経由でのみ参照されていたが、本変更ではコマンド自体の削除は行わない（後述「リスクと代替案」参照）。`FileStatus`（`src/types/file-tree.ts`）型は `FileNode.status` が参照するため残す。

## アーキテクチャと責務分割

```
ReviewPanel (view)
  └─ useReviewSnapshot (frontend / 描画専従: invoke + 保持 + 公開のみ)
       └─ invoke("get_review_snapshot")
            └─ adaptor/controller/command/code/review.rs (controller)
                 └─ usecase/review_usecase.rs (split decision の所有者)
                      ├─ head_review_snapshot / branch_base_review_snapshot
                      └─ split_staged_changed (共通ヘルパー)
                           └─ code_dto::ReviewSnapshotDto (read model / transport)
```

- **split decision の所有者**: `usecase/review_usecase.rs`。`index_status != "none"` → staged、`worktree_status != "none" && != "ignored"` → changed の規則を Rust が単独で持つ。
- **read model**: `code_dto::ReviewSnapshotDto`。backend-owned で、Tauri / 将来の WebSocket・daemon client が同じ camelCase shape を読める。
- **frontend (`useReviewSnapshot`)**: invoke 結果の保持・version dedup・race 制御・公開に責務を限定。staged / changed の分類判断を持たない。
- **view (`ReviewPanel`)**: 供給された `stagedFiles` / `changedFiles` 集合を表示・選択判定に使う。変更なし（供給元が backend 集合へ変わる配線のみ）。

## データモデルまたは型

### backend: `ReviewSnapshotDto`（公開フィールド）

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotDto {
    // ... 既存フィールド（version, stale, loading, limited, base, files,
    //     diff_stats, tree, staged_tree, changes_tree,
    //     staged_file_count, changes_file_count）...
    pub staged_files: Vec<FileStatusDto>,   // -> stagedFiles
    pub changed_files: Vec<FileStatusDto>,  // -> changedFiles
}
```

`FileStatusDto`（`{ path, index_status, worktree_status }`）は frontend `GitFileStatus`（`{ path, index_status, worktree_status }`）と shape / 値域を一致させる。`status` 全件フィールドは transport に含めず、ReviewPanel が必要とする公開集合は `staged_files` / `changed_files` のみとする。

branch-base 経路の元データである `ChangedFileDto.status` は branch diff 用の値域（例: `added` / `copied` / `renamed`）を持つため、`FileStatusDto.worktree_status` へ入れる前に repository status の値域へ正規化する。具体的には `added` / `copied` → `new`、`renamed` → `modified`、`modified` → `modified`、`deleted` → `deleted` とする。`ReviewFileEntryDto.worktree_status` や `DiffTreeNodeDto.status` は Review / diff 表示用の string として branch diff status を保持できるが、`staged_files` / `changed_files` に入る値は `GitFileStatus` 契約に揃える。

### split ヘルパー

```rust
// review_usecase.rs（module 内 private fn）
fn split_staged_changed(statuses: &[FileStatusDto]) -> (Vec<FileStatusDto>, Vec<FileStatusDto>) {
    let staged = statuses
        .iter()
        .filter(|e| e.index_status != "none")
        .cloned()
        .collect();
    let changed = statuses
        .iter()
        .filter(|e| e.worktree_status != "none" && e.worktree_status != "ignored")
        .cloned()
        .collect();
    (staged, changed)
}
```

`staged_file_count` / `changes_file_count` は `staged.len()` / `changed.len()` から導出する（既存の count 算出を集合長へ置換するだけで、規則・値は不変）。

### frontend: `ReviewSnapshot`（追加フィールド）

```ts
export interface ReviewSnapshot {
	// ... 既存 ...
	stagedFiles: GitFileStatus[];
	changedFiles: GitFileStatus[];
}
```

## 処理フロー

1. `useReviewSnapshot` が `invoke("get_review_snapshot", { input: { worktreePath, base } })` を呼ぶ（既存どおり）。
2. usecase が `base` に応じて `head_review_snapshot`（`base = head`）または `branch_base_review_snapshot`（`base = branch-base`）を構築する。
   - `head_review_snapshot`: repository snapshot 内部の status 全件（ignored 含む）に対し `split_staged_changed` を適用。staged = `index_status != "none"`、changed = `worktree_status != "none" && != "ignored"`。
   - `branch_base_review_snapshot`: branch diff の変更全件から内部用の `FileStatusDto` 配列（各要素 `index_status = "none"`、`worktree_status = 正規化済み branch diff status`、ignored を含まない）を構築し、同じヘルパーを適用。結果は staged = 空、changed = 変更全件となり、従来の frontend filter 結果と一致しつつ `GitFileStatus` の値域契約を満たす。
3. `ReviewSnapshotDto` に `staged_files` / `changed_files` を詰めて返す。`status` 全件フィールドは transport に含めない。
4. `useReviewSnapshot` は version dedup・race 制御（既存）を通過後、snapshot を保持。`stagedFiles` / `changedFiles` を `visibleSnapshot` からそのまま返す（filter なし）。
5. 入力キー不一致時・取得前は `EMPTY_SNAPSHOT`（`stagedFiles: []` / `changedFiles: []`）を返す（既存の `visibleSnapshot` フォールバックがそのまま機能）。
6. `ReviewPanel` は受け取った集合で従来どおり描画・選択判定する。

## エラー処理

- usecase は既存の `Result<ReviewSnapshotDto, CodeUsecaseError>` を踏襲。split ヘルパーは純粋関数で失敗経路を持たない（fallible 化しない）。branch-base の status 正規化で未対応の branch diff status を受け取った場合は、契約不一致として `CodeUsecaseError` を返す。
- `useReviewSnapshot` の `catch` は既存どおり `EMPTY_SNAPSHOT`（`stagedFiles` / `changedFiles` 空）へフォールバック。`rootPath` 無しの早期 return も同様に空。これにより behavior の「取得不可時は staged / changed を空」を満たす。
- version dedup（厳密に古い version の snapshot を反映しない）・race 制御は変更しない。

## テスト方針

### Rust（split 規則の担保）

`review_usecase.rs` の `#[cfg(test)]` に split を検証する test を追加する。`split_staged_changed` を直接、または `head_review_snapshot` 経由で検証する。

- staged-only（`index_status = modified`, `worktree_status = none`）→ staged のみ。
- changes-only（`index_status = none`, `worktree_status = modified`）→ changed のみ。
- both（`index_status = new`, `worktree_status = deleted`）→ staged かつ changed の両方。
- ignored（`worktree_status = ignored`）→ どちらにも含まれない。
- clean（`none`/`none`）→ どちらにも含まれない。
- `staged_file_count` / `changes_file_count` が各集合長と一致すること（既存 count 挙動の退行防止）。
- `branch_base_review_snapshot` 経路で staged = 空、changed = 変更全件となること。
- branch-base 経路の `FileStatusDto.worktree_status` が branch diff status（`added` / `copied` / `renamed`）を repository status 値域（`new` / `modified` / `deleted`）へ正規化すること。

これは ISSUE #1303 完了条件のうち「staged / unstaged split の検証」に対応する。overlay 系（file status propagation / directory aggregation / unknown fallback）の Rust test は、当該ロジックを削除するため対象外（requirements 合意済み）。

### frontend（描画・loading・error へ寄せる）

- `useReviewSnapshot.test.ts`: mock invoke が `stagedFiles` / `changedFiles` を含む snapshot を返すとき、フックがそれをそのまま公開し、`status` から再計算しないこと。`rootPath = null` / 取得失敗で両集合が空になること。version dedup の既存挙動が退行しないこと。
- `ReviewPanel.test.tsx`: `useGitStatus` mock 削除後も既存テスト（staged/changes 分類・diff path 供給・選択判定）が `useReviewSnapshot` mock の `stagedFiles` / `changedFiles` で従来どおり通ること。
- 削除した分類規則の単体テスト（`useGitStatus.test.ts` / `applyStatusToTree.test.ts`）は除去。

### 受け入れ確認

- frontend grep で staged/changed split の filter（`index_status` / `worktree_status` の `!== "none"` 振り分け）が `useReviewSnapshot` に残っていない。
- `useGitStatus.*` / `applyStatusToTree.*` が存在しない。`ReviewPanel.test.tsx` に `useGitStatus` mock が無い。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## リスクと代替案

- **`status` 全件フィールドの削除**: `ReviewSnapshotDto` は `staged_files` / `changed_files` を公開集合として持ち、`status` 全件フィールドは transport に含めない。同一データの重複表現を read model に残さず、frontend の filter 再計算経路も復活させない。
- **`get_git_status_snapshot` コマンドの残置**: `useGitStatus` 削除で唯一の frontend 消費者が消えるが、本 ISSUE のスコープは status の staged/changed read model 化に閉じる（requirements 非スコープ）。コマンド・backend read model の削除可否判断は #878 final dead-code sweep に委ねる。
- **代替案: `staged_files` / `changed_files` を path 文字列配列にする**。ReviewPanel が `f.path` のみ参照するため path 配列でも足りるが、`GitFileStatus` 集合のままにすることで `useReviewSnapshot` の公開型（`GitFileStatus[]`）と既存テスト・将来の status 表示拡張に対する後方互換を保つ。本設計は集合（`FileStatusDto` / `GitFileStatus`）を採用する。
- **代替案: 既存 `staged_file_count` を集合長導出に変えず別計算で残す**。冗長なため集合長から導出して一本化する。値・規則は不変。

## 仮定

- usecase 内部で split の入力にする status 配列は ignored を含む全件（`head` 経路）または正規化済み status を持つ変更全件（`branch-base` 経路）であり、`split_staged_changed` を適用すれば従来 frontend filter と同一の集合が得られる。
- ReviewPanel が staged / changed に対して参照するのは各要素の `path`（および将来的に status 値）であり、`FileStatusDto` の clone 供給で観測結果は不変。
- `FileStatus`（`src/types/file-tree.ts`）型は `FileNode.status` が参照するため本 ISSUE では削除しない（#878 に委ねる）。
- version / stale / loading / limited の意味・採番、`useReviewSnapshot` の version dedup・race 制御は不変。

## Open Questions

なし。
</content>
