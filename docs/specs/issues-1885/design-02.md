# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1885` の `42be41e0`。直前の Design は `docs/specs/issues-1885/design-01.md`。
- 開始状態の実装は、`42be41e0` に design-01 の変更を適用した未コミットの作業ツリーである（152 ファイル）。購読の土台は `SubscriptionTarget`（`src-tauri/src/domain/state_subscription/subscription_target.rs`）、`Subscriptions`（`mod.rs`）、`WorkspaceStateReads`（`src-tauri/src/usecase/state_subscription/reads.rs`）、client 側は `useStateSubscription`（`src/hooks/useStateSubscription.ts`）と `subscribeState`（`src/lib/client.ts`）にある。design-01 の「変える部分」は実装済みであり、この Design では再掲しない。
- この周までに解消・見送りとなった Thread は無い。open Thread は 13 件で、いずれも `[FIX_POLICY]` が付いている。`[DEFERRED]` と `[REJECTED]` は 0 件である。
- Requirements と Behavior は今周で変更していない。対応表は R-001〜R-016 と B-001〜B-023 を漏れなく含み、誤り・不足・矛盾は見つからなかった。

## 変える部分

Requirements・Behavior の変更は無い。以下は `[FIX_POLICY]` が付いた open Thread 13 件に対応する変更である。

- workflow execution の archive / restore での購読の更新: inactive な workflow execution の archive と restore が成功した後、client の取り直しなしで Workspaces のツリーの表示が更新後の内容になるようにする。開始状態では `src-tauri/src/adaptor/controller/client/dispatch.rs:120-143` の `StateChangeSource` の対応に `archive_workspace_workflow_execution` / `restore_workspace_workflow_execution` の arm が無く既定の `None` になり、`src-tauri/src/usecase/workflow/workspace_tree.rs:257,267` にも購読への通知が無い。根拠: R-004「Workspaces の表示は、client からの取り直しの呼び出しを行わずに、daemon の状態の変化を反映する」、B-004、Thread `d63a3739-b1dd-4259-ab58-36cf7cd5ab22`。ルート: 委任。
- 状態を変える操作と購読の更新対象の対応の所有者の移動: 対応を controller の分岐から domain / usecase へ移す。開始状態では `dispatch.rs:120-143` が wire command 名から `StateChangeSource` への対応表を持つ。R-004 / B-004 の観測可能な振る舞いは変えない。根拠: `docs/architecture/CONTROLLER.md`「業務ロジックを書かない（Usecase を呼ぶだけ）」、`docs/architecture/DOMAIN.md`「判断・計算・分類・検証・方針は domain にある」、Thread `e497db9a-0839-48d3-bc7f-3d77f3ea97b0`。ルート: 委任。
- 購読の開始が失敗したときの対象の解放: 購読の開始が `StreamEnded` で失敗したとき、その対象が `Subscriptions` の `targets` に残らないようにする。開始状態では `src-tauri/src/usecase/state_subscription.rs:132-146` が `register` の後に `state.start` を呼び、失敗時の `release_inactive_snapshots`（`src-tauri/src/domain/state_subscription/mod.rs:265-273`）は `snapshot` と `history` だけを解放して `targets` の鍵を残す。daemon の起動時に登録される `RepositoryPaths` は従来どおり残す。根拠: `AGENTS.md`「full-retention 設計を避ける」とレビュー観点「full-retention / full-recompute 経路を増やしていないか」、Thread `e6493169-992f-4c9c-b068-792198186dd8`。ルート: 委任。
- `RepositoryStateError` の到達経路を失った変換の削除: `src-tauri/src/adaptor/controller/client/repository/mod.rs:17` の import と `:28-32` の `From<RepositoryStateError> for AppError` を無くす。design-01 で `run_repository_state` を削除したため、この controller から到達する経路は無い。gateway / usecase 側の利用は変えない。根拠: R-015、B-016、Thread `c9d5bed5-34a5-4d25-bdaa-7888d60e1bf1`。ルート: 委任。
- 本文のない条件分岐の削除: `src/components/workspace/WorkspaceList.tsx:905-906`、`:909-910`、`:1128-1129`、`:1277-1278` に残る本文のない `if` / `else` を無くす。design-01 の取り直し処理の削除で本文だけが消えたものである。根拠: R-015、B-016、Thread `5e88028a-726d-42db-a3a8-cec90a8e2e38`。ルート: 委任。
- `workspace-state` の購読の失敗で表示が固着する点の修正: 購読の開始や継続が失敗しても `stateReady` が完了へ遷移し、未処理の Promise 拒否が発生しないようにする。開始状態では `src/hooks/useWorkspaceStateCache.ts` の `loadState` が `subscribeState` の `onError` を Promise の `reject` へ直結し、唯一の呼び出し元 `src/hooks/useWorkspacePersistence.ts:123-127` は `catch` を持たない。`42be41e0` の `loadState` は失敗を握りつぶして `undefined` で解決していた。取り直しと再試行は足さない。根拠: R-005 が削除を求めるのは取り直しと再試行であること、R-001（対象のうち「worktree ごとの保存済み表示状態」）、B-001、B-005、Thread `8bc17971-8f96-40ca-b8cf-735ccfa95e3d`。ルート: 委任。
- `set_branch_base` の失敗の扱いの修正: 失敗しても未処理の Promise 拒否が発生しないようにする。開始状態では `src/hooks/useBaseBranch.ts:22-30` が `invoke("set_branch_base")` を `catch` 無しで返し、`src/screens/MainLayout.tsx:377` から void callback として渡る。`42be41e0` には `catch` があった。失敗時に client から取り直しの呼び出しは行わない。根拠: R-005 が削除を求めるのは失敗を受けての再取得であること、B-005、R-004 / B-004（変更後の状態は購読で届く）、Thread `8fb6024b-8a9e-4b41-a6ef-f7a024722ca5`。ルート: 委任。
- `desktop-bundle` テストの移行漏れの解消: `tests/desktop-bundle.mjs:46,55,60,62` が削除済みの `get_workflow_execution_state` を呼ぶのをやめ、window を閉じてから再表示するまでの workflow の存続確認を成立させる。`proto/client.proto:25,209` はこの command を reserved にしている。根拠: R-002、B-002、R-015、B-016、Thread `06f4c0a8-448f-4487-bb8c-ddccb5329052`。ルート: 委任。
- `WorkspaceStateReads` の振り分けの検証: 各購読対象が対応する usecase / query service を呼んで値を組み立てることを usecase 層のテストで確認できるようにする。開始状態では `src-tauri/src/usecase/state_subscription/reads.rs` に `cfg(test)` も専用のテストファイルも無い。根拠: R-001、R-003、B-001、B-003、`docs/architecture/TEST.md` の usecase 必須、Thread `54ebf5a1-fb9a-4b72-bbe2-b09bdda7f621`。ルート: 委任。
- `Branches` の更新配信の検証: `Branches` の対象を購読した状態で `StateChangeSource::Repository` が起きるとその対象が更新対象として選ばれ、変わった後の branch の一覧が届くこと、および `Branches` が git の監視を必要とする対象として扱われることを domain のテストで確認できるようにする。開始状態では `subscription_target_test.rs` は往復と拒否の 2 件だけ、`subscriptions_test.rs` の監視のテストは `Workspaces` だけを使う。根拠: R-012、B-015、`docs/architecture/TEST.md` の domain 必須、Thread `72bb28dc-d4fb-4907-9103-8bfbcf02793f`。ルート: 委任。
- 利用者の取り直しの要求からの即時配信の検証: active な issue の購読がある状態で `fetch_issues` を実行すると、30 秒の到達を待たずに外部取得が行われ、更新後の issue の一覧が同じ購読へ change として届くことを自動テストで確認できるようにする。開始状態では `dispatch_test.rs` は `FetchIssues` 成功時に `StateChangeSource::Issues` が送られるところまでしか通していない。根拠: R-008、B-017、`docs/architecture/TEST.md` の usecase 必須、Thread `ee595ec0-96af-40c4-9568-90ca866cd423`。ルート: 委任。
- 購読エラー時の直前値の保持の検証: 同じ target で購読エラーが起きたときは直前の値が保たれ、target が変わったときは値が `undefined` になることを hook のテストで確認できるようにする。開始状態では `src/hooks/useStateSubscription.test.ts` は存在しない。この保持は `src/hooks/useWorkspaceList.ts:17` 経由で一覧の保持を支えている。根拠: R-009、B-012、`AGENTS.md` のフロントエンドのテスト方針、Thread `8c5815b1-36fb-448c-94a0-2c790ef6e151`。ルート: 委任。
- 起動時の 1 件の自動表示の検証: `startup-repository` が repository のルートを返し、`worktrees` が 1 件を返したとき、その 1 件が worktree のタブとして開かれることを App のテストで確認できるようにする。開始状態では `src/App.tsx:162-185` がこの経路を持つが、`src/App.workspace-archive.test.tsx:235-236` は `worktrees` を空配列で publish しており 1 件の分岐を通すテストが無い。根拠: R-014、B-022、R-001（対象のうち「worktree の一覧」）、B-001、`AGENTS.md` のフロントエンドのテスト方針、Thread `d60d7e5d-069e-4671-b2e3-7ac4ba69bd1f`。ルート: 委任。

## 固定するルート

今周に新しく人間が指定したルートは無い（`materials.design.directions` は空）。design-01 の「固定するルート」1〜5 は今周も維持し、解除・変更しない。

1. 購読対象を表す値オブジェクトを `domain/state_subscription` に置き、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵をその型にする（design-01 の 1）。
2. その値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う（design-01 の 2）。
3. 外部情報の鮮度は `domain/git_host` の `CacheTtl` を唯一の規則とし、取り直しの間隔を別に持たない（design-01 の 3）。
4. worktree と session から引き当てた Node の id、client が渡した任意のパスから解決した repository のルート、daemon の起動ディレクトリから解決した repository のルートは、いずれも購読の対象にする（design-01 の 4）。
5. agent session の履歴は、購読の対象を「表示している件数」で定義する。件数を増やすときは購読を張り直す（design-01 の 5）。

## 変えないもの

design-01 の「変えないもの」をそのまま維持する。今周で人間が新しく指定した維持の条件は無い。

- Workspaces 行の Refresh ボタンと、一覧の保持・失敗した範囲の表示・初回の取得の失敗と「項目なし」の区別。
- 旧 Push の枠組み（`SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`）と監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）。
- Review と Automation が要求するファイル監視（`start_watching`）。
- Settings の他の設定の読み取り（`get_releash_base`、`get_external_editor`、`get_workflow_config` など）。
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-007 と B-009 の「client は監視の開始と停止を要求しない」を、Workspaces の表示のための監視に限ると解釈して Requirements と Behavior を修正した。詳細は `requirements.md` の Assumptions を参照する。今周で新たに自動判断した箇所は無い。
- `[DEFERRED]` で人間へ渡した件: 無し。
- 未決のまま残した要求: 無し。
