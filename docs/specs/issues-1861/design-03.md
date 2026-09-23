# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01・Design 02 の周の実装は未コミットの作業ツリーにあり、Spec 工程ではコードを変更していないため、この実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-02.md`。
- この周までに解消・見送りとなった Thread はない。open Thread は5件で、5件とも `[FIX_POLICY]` が付き、`[REJECTED]`・`[DEFERRED]` はない。
- この周の前段で Requirements に R-016 が、Behavior に B-022・B-023 と対応表の R-016 行が追加されている。R-001〜R-015 と B-001〜B-021 は変更されていない。Requirements の Assumptions / Open Questions は「なし」である。
- Design 02 の「未確定・リスク」2件は開始状態で解消している。`rescan_branches` は `commit_snapshot` に `notify_snapshot_changed` を伴わせないため、更新が自身の再実行を呼ばない（`src-tauri/src/usecase/repository_state/service.rs:107-121`）。局所操作後の再読込 `refresh_worktree` は `begin_worktree` を通り、全体更新と同じ `WorkspaceLists` の generation 管理へ入る（`src-tauri/src/usecase/workspace_tree/list.rs:134-140`）。

## 変える部分

- 明示再走査の失効結果を公開しない: 明示的な再走査の実行中に `invalidate` で generation が進んだ場合、その走査結果を `WorktreeState` へ公開しない。開始状態では `rescan_branches` が走査前に `requested_generation` を読み、走査完了後に再照合せず `commit_snapshot(parts, generation)` を呼ぶ（`service.rs:107-121`）。`invalidate` は走査中でも `requested_generation` を進め（`worktree.rs:192-203`）、`commit_snapshot` は generation を照合せず snapshot を上書きする（`worktree.rs:214-223`）。根拠: Thread ce4279b6-2388-426d-9538-39d3edb26bba（blocking）、R-013「自動更新と手動更新が重なり応答の順序が入れ替わっても、より古い取得結果でより新しい一覧を上書きしない」、B-017。ルート: 委任
- `refresh_workspaces` の dispatch の行き先を controller の実経路で確認できるようにする: `worktreePath` を指定した要求が局所更新へ、指定しない要求が全体更新へ到達することを判別できるようにする。開始状態では `workspace_tree_shared.rs:25-27` に `Some`／`None` の分岐があるが、`protocol/client/client_test.rs:485-494` は protobuf 往復と型、`command/client_test.rs:357-358` はコマンド名の登録だけを確認し、行き先を確認するものがない。根拠: Thread ff0fafd5-7a07-4e91-bc88-5a892293cf6d、R-005「折りたたまれている Repository も対象に含み、表示部品の再作成に依存せずに更新する」、B-006、design-02.md の「局所操作後の再読込の対象範囲の是正」。ルート: 委任
- 参照されない更新中 state の除去: `WorkspaceListModel.loading` と `pendingRef` を取り除く。開始状態では `useWorkspaceList.ts:29・34・36-37・48-49` が request ごとに両者を更新し `:128` で公開するが、読む本番コードはなく（消費者は `WorkspaceList.tsx:1706-1723` の `snapshot`・`error`・`status.loaded`、`useWorkspaceTreeNodes.ts:81-86` の `snapshot`・`refreshWorktree`）、手動更新の進行表示と重複抑止は `WorkspaceList.tsx:1707-1719` の `refreshing`・`refreshingRef` が担う。根拠: Thread 4359c61e-2509-4de4-a645-49f2bccf8774、R-012「手動更新の進行中は、進行していることを示し、同じ更新の重複した要求を受け付けない」、B-016、`AGENTS.md`「frontend state は UI に必要な状態の mirror に留め、domain behavior の source of truth にしない」。ルート: 委任
- `WorkspaceListUsecase` の構築の composition root への集約: daemon と desktop の両入口が同じ構築手順から `WorkspaceListUsecase` を受け取るようにする。開始状態では `daemon.rs:257-264` と `client/workflow/mod.rs:1321-1328` に `WorkspaceListUsecase::new` と `WorkspaceListServices`（`repositories`・`repository_state`・`workflow`・`git_host`）の同じ組立てが重複し、後者は同じ関数内で `wiring::build_git_host_usecase` を経由しているのに `WorkspaceListUsecase` だけが composition root を通らない。根拠: Thread 20465e88-a8e3-467d-8d84-e81f5c1e55e0、`wiring.rs:1-9`「gateway 実装を repository / usecase へ合成する組み立ては controller の責務であり、gateway 層や各エントリポイントへ漏らさない」、`AGENTS.md`「入口は3つあり、同じ usecase を共有する」。ルート: 委任
- 登録 Repository 一覧の参照先の一本化: 登録 Repository 一覧を表示する画面が参照する一覧を `refresh_workspaces` の snapshot へ寄せ、これによって不要になる `useRepoList` の読み取り側（`repoPaths`・`loaded`・`loadError` の3 state、初回 `get_repo_paths` 呼び出し、`repo-paths-changed` listener）を除去する。`useRepoList` には `addRepo`・`removeRepo`・`initFromCwd` を残す。開始状態では `App.tsx:113-119` が `useRepoList` から取った `repoPaths` を `:332` で設定画面へ渡し、`useRepoList.ts:19-33` は初回 `get_repo_paths` の失敗後に再試行せず `repo-paths-changed` だけを待つ一方、`WorkspaceList.tsx:1720-1721` は snapshot 由来の `listedRepoPaths` を使うため両者が分岐する。根拠: Thread dac577a2-41c5-4d1b-a676-8b7682f0aa6d、R-016「登録 Repository 一覧を表示する画面は、Workspaces が表示している登録 Repository 一覧と同じ Repository を表示する。Workspaces の更新によって登録 Repository 一覧が変わった場合も一致する」、B-022、B-023。ルート: 委任

## 固定するルート

この周で新しく固定する実装上の指定はない。Design 01 で固定した次の4つを維持する。

- 全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。（Design 01 固定ルート①）
- 一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。（Design 01 固定ルート②）
- 更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。（Design 01 固定ルート③）
- 手動更新と自動更新で内部の経路を分けない。（Design 01 固定ルート④）

## 変えないもの

- 登録 Repository 一覧の参照先の一本化の対象範囲。Rust 側の `refresh_workspaces` の取得内容、更新手順、結果の判定は変えない。理由: 人間がこの Thread の対応範囲を frontend の参照先の変更に閉じると定めたため。
- `src/App.tsx:120-128` の起動時挙動。登録 Repository 一覧の初回取得に失敗しても restoration を完了する挙動を取り消さない。理由: 人間がこの Thread を取り消しではなく修正と決めたため。
- 取得状態の3区別（R-010、B-012・B-013・B-014）の適用範囲。Workspaces に限り、登録 Repository 一覧を表示する画面へ広げない。理由: 人間が適用範囲をそう定めたため。
- R-012・B-016 の進行表示と重複抑止、および R-001・B-001 の一覧保持の観測結果。参照されない更新中 state を除いても変えない。理由: 人間が該当 Thread の受入条件をこれらの観測結果が変わらないことと定めたため。

## 未確定・リスク

- 失効した走査結果を公開しない場合に `WorkspaceListQueryService::branches` が何を返すかが未確定である。開始状態では `branches` が `rescan_branches` の結果をそのまま返し（`list_query_service.rs:41-46`）、`Err` は `complete_branches` を通って Repository 階層の取得失敗になる（`list.rs:99-132`）。失効を取得失敗として返すと、実際には取得に失敗していない Repository に R-009・B-010 の更新失敗が現れる。
- 登録 Repository 一覧の参照先を `refresh_workspaces` の snapshot へ寄せる場合、その結果を読む場所が `WorkspaceListContext` の provider の外側にある。provider は `WorkspaceList.tsx:1726` の内側にあり、`App.tsx:121-128` の restoration の完了判定と `:327-335` の設定画面への受け渡しは provider の外側にある。開始状態の完了判定は `useRepoList` の `loaded`・`loadError` が確定すれば成立するが、差し替え先が取得の成功と失敗のどちらでも確定しないと restoration が完了せず、R-004・B-005 の「一覧の取得に失敗している状態でも Workspaces 行の更新の操作を実行できる」状態へ到達しない。
- この周で自動判断した箇所はない。Requirements と Behavior の対応（R-001〜R-016 と B-001〜B-023、対応表の全 Requirement ID の記載）に誤り・不足・矛盾は見つからず、両文書を変更していない。未決のまま残した要求はなく、`[DEFERRED]` で人間へ渡した件もない。
