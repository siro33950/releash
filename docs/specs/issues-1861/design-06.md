# Design 06

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01〜05 の周の実装は未コミットの作業ツリーにあり、Spec 工程ではコードを変更していないため、この実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-05.md`（入力の `context.design_path` と一致）。
- open Thread は2件（7b869874-f28d-42dc-8c13-b6f73e98790f、25fb868b-ced6-4bcf-afe9-4b7515afceaf）で、いずれも `[FIX_POLICY]` が付いている。`[REJECTED]`・`[DEFERRED]` にした Thread はない。前の周までに扱った Thread（dac577a2、ff0fafd5、4359c61e、20465e88、ce4279b6、1bd38ae7、1e36eb0c、ee6cc9df）は開始時点で open ではない。
- Requirements・Behavior はこの周で変更していない。R-001〜R-016 と B-001〜B-023 に欠番・重複がなく、対応表が全 Requirement ID を記載し各行に Behavior ID が対応していることを確認した。Assumptions / Open Questions は「なし」である。
- Design 05 の「変える部分」1件は開始状態で満たされている。`useWorkspaceList` は `snapshot`・`error`・`worktreeErrors`・`refresh`・`refreshWorktree`・`refreshRepository` を依存値とする `useMemo` で `WorkspaceListModel` を返し、これらが変わらない render では参照を維持する（`src/hooks/useWorkspaceList.ts:164-181`）。
- Design 05 の「未確定・リスク」は「なし」であり、引き継ぐ事項はない。

## 変える部分

- Repository を対象とする再読込の失敗の保持先: Repository を対象とする `refresh_workspaces` の RPC が失敗したとき、その失敗を対象 Repository の失敗として示し、Workspaces 一覧全体と取得に成功している他の Repository には示さない。開始状態では `src/hooks/useWorkspaceList.ts:71-79` の catch が `worktreePath` の有無だけで分岐するため、`refreshRepository`（`:92-95`、`request(undefined, repoPath)`）の失敗が `setError` へ入り、`src/components/workspace/WorkspaceList.tsx:1724`（`model.error ?? model.snapshot?.status.error`）と `:1762` を通って Workspaces 一覧全体の失敗として表示される。Repository 単位の失敗を保持する frontend の state はなく、Repository 行が表示する `:1655-1656` の `status.error` は Rust 側 snapshot 由来の値だけである。根拠: Thread 25fb868b-ced6-4bcf-afe9-4b7515afceaf（blocking、`[FIX_POLICY]`）、R-009「取得に失敗した Repository・Worktree には、その一覧の更新に失敗したことと、表示しているのが前回取得の情報であることを示す」、B-010、B-020。ルート: 委任
- Workspaces 一覧全体の失敗表示の解消条件: Workspaces 一覧全体に更新失敗が示されている状態で、Worktree または Repository を対象とする再読込だけが成功しても全体の失敗表示を残し、登録 Repository 一覧を対象に含む更新が成功したときに解消する。開始状態では `src/hooks/useWorkspaceList.ts:69` が `worktreePath`／`repoPath` の有無にかかわらず RPC 成功時に `setError(null)` を実行する一方、Rust 側で登録 Repository 一覧を再取得するのは `src-tauri/src/usecase/workspace_tree/list.rs:83-97` の `refresh` だけであり、`refresh_repository`（`:99-105`）と `refresh_worktree`（`:142-148`）は再取得しない。根拠: Thread 7b869874-f28d-42dc-8c13-b6f73e98790f（blocking、`[FIX_POLICY]`）、R-011「再取得に成功した対象では、それまでの更新失敗の表示を解消する」、B-015、R-009 後段、B-020。ルート: 委任

## 固定するルート

この周で新しく固定する実装上の指定はない。Design 01 で固定した次の4つを維持する。

- 全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。（Design 01 固定ルート①）
- 一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。（Design 01 固定ルート②）
- 更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。（Design 01 固定ルート③）
- 手動更新と自動更新で内部の経路を分けない。（Design 01 固定ルート④）

## 変えないもの

- B-020 の Workspaces 一覧全体への失敗表示。登録 Repository 一覧そのものの取得に失敗した場合は、これまでどおり Workspaces 一覧全体に更新失敗と前回取得の情報であることを示す。理由: Thread 25fb868b・7b869874 の `[FIX_POLICY]` が受入条件として維持を定めたため。
- R-001〜R-016 と B-001〜B-023 の受入条件。失敗の保持先と解消条件の対象範囲を正すだけで、一覧の保持、進行表示と重複抑止、新旧判定の観測結果は変わらない。理由: 2件の Thread の `[FIX_POLICY]` がいずれも既存の要求・受入条件を根拠としており、要求・受入条件の追加・変更・緩和を伴わないため。

## 未確定・リスク

- この周で自動判断した箇所はない。Requirements と Behavior の対応に誤り・不足・矛盾は見つからず、両文書を変更していない。未決のまま残した要求はなく、`[DEFERRED]` で人間へ渡した件もない。
