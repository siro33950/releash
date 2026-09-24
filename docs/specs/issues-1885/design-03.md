# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1885` の `42be41e0`。直前の Design は `docs/specs/issues-1885/design-02.md`。
- 開始状態の実装は、`42be41e0` に design-01 と design-02 の変更を適用した未コミットの作業ツリーである（171 ファイル）。design-01 と design-02 の「変える部分」は実装済みであり、この Design では再掲しない。
- この周までに解消・見送りとなった Thread: design-02 が扱った 13 件のうち 12 件（`d63a3739`、`e497db9a`、`c9d5bed5`、`5e88028a`、`8bc17971`、`8fb6024b`、`06f4c0a8`、`54ebf5a1`、`72bb28dc`、`ee595ec0`、`8c5815b1`、`d60d7e5d`）は resolved である。残る `e6493169` は open のままである。`03b24421`（Workspaces 購読の初回開始と Repository の追加・削除で Repository 一覧自体を取り直さないという指摘）は不成立として `[REJECTED]` で resolve された。`[DEFERRED]` は 0 件である。
- open Thread は 5 件（`e6493169`、`88a01439`、`623a47b2`、`bf4891b6`、`96fd3ba4`）で、いずれも `[FIX_POLICY]` が付いている。
- Requirements と Behavior は今周で変更していない。対応表は R-001〜R-016 と B-001〜B-023 を漏れなく含み、欠番・重複・参照切れは無い。誤り・不足・矛盾は見つからなかった。

## 変える部分

Requirements・Behavior の変更は無い。以下は `[FIX_POLICY]` が付いた open Thread 5 件に対応する変更である。

- 購読の開始が失敗したときの対象の解放: 購読の開始が `StreamEnded` で失敗したとき、その対象が `Subscriptions` の `targets` に残らないようにし、その解放を検証するテストを置く。design-02 が同じ変更を挙げたが開始状態では未実施であり、`src-tauri/src/usecase/state_subscription.rs:136-149` は `start_with_snapshot` の後に活性な対象でないことを確かめて `StreamEnded` を返すだけで、`src-tauri/src/domain/state_subscription/mod.rs:285-293` の `release_inactive_snapshots` は `RepositoryPaths` 以外の非活性な対象の `snapshot` と `history` を空にして `targets` の鍵を残す。daemon の起動時に登録される `RepositoryPaths` は従来どおり残す。根拠: `AGENTS.md`「full-retention 設計を避ける」とレビュー観点「full-retention / full-recompute 経路を増やしていないか」、Thread `e6493169-992f-4c9c-b068-792198186dd8`。ルート: 委任。
- `Providers` の更新配信の検証: provider の一覧の状態が変わったとき、`Providers` の対象が更新対象として選ばれ、変わった後の一覧が同じ購読へ届くことを domain のテストで確認できるようにする。開始状態では `SubscriptionTarget::affected_by` の `C::Providers` 分岐（`src-tauri/src/domain/state_subscription/subscription_target.rs:195`）を通すテストが無く、`subscriptions_test.rs` の `"providers"`（`:9,53,119,127,133`）は汎用の publish 経路へ渡す文字列として使われているだけで、`src-tauri/src/usecase/state_subscription/reads_test.rs:281` は `T::Providers` の初回読み取りだけを通している。根拠: R-001、R-004、B-001、B-004、`docs/architecture/TEST.md` の domain 必須、Thread `88a01439-2886-402f-88af-33615e3268c3`。ルート: 委任。
- 任意のパスからの repository のルートの解決の検証: 任意のパスを指定した `repository-root` の購読が成功したときにそのパスから解決した repository のルートが Repository として追加されること、および購読が失敗したときに選んだパスが worktree のタブとして開かれることを App の component テストで確認できるようにする。開始状態では `src/App.tsx:187-198` の `handleAddRepo` がこの両分岐を持つが、`src/App.test.tsx`、`App.restoration.test.tsx`、`App.startup-failure.test.tsx`、`App.workspace-archive.test.tsx` のいずれにも `repository-root` の対象と Add Repository の操作を通すテストが無い。根拠: R-013、B-021、`AGENTS.md` のフロントエンドのテスト方針「component は user interaction と conditional rendering をテストする」、Thread `623a47b2-ec23-4a9e-8097-17b443244a95`。ルート: 委任。
- `WorkspaceList` のテストが呼ぶ削除済みの読み取りの除去: `src/components/workspace/WorkspaceList.test.tsx:132` の `useWorkspaceTreeNodes` の mock が `archivedSessions` 未指定時に `list_workspace_worktree_nodes` を `mocks.invoke` へ渡し、`:377`、`:1692`、`:2115`、`:2185` がそれを処理しているのをやめ、`archivedSessions` を購読で届く snapshot に相当する経路から与えて同じ検証が成立するようにする。`proto/client.proto:29,213` はこの command を reserved にしている。`src-tauri/src/adaptor/controller/command/client_test.rs:344,1342` の同名の文字列は削除済みの command が登録されていないことを確かめる一覧であり対象外とする。根拠: R-015、B-016、Thread `bf4891b6-67de-4b8c-bb51-ca1209761e50`。ルート: 委任。
- 購読対象名のエンコードの所有者の一本化: 対象名の組み立て規則が domain（および対象名との相互変換を担う adaptor）だけに存在し、client が同じ規則を自前で組み立てないようにする。開始状態では `src-tauri/src/domain/state_subscription/subscription_target.rs` の `Display` が対象名と引数ごとの `{len}:{arg}` の連結を持ち `parse` が同じ形式の解釈と引数の検証を持つ一方、`src/lib/client.ts:250-256` の `stateTargetKey` が `TextEncoder` で同じ形式を別に組み立てている。根拠: 「固定するルート」1（design-01 の 1、design-02 で維持）、Thread `96fd3ba4-6afa-49ad-abae-9a2d289fbb34`。ルート: 委任。

## 固定するルート

今周に新しく人間が指定したルートは無い（`materials.design.directions` は空）。design-01 の「固定するルート」1〜5 は今周も維持し、解除・変更しない。

1. 購読対象を表す値オブジェクトを `domain/state_subscription` に置き、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵をその型にする（design-01 の 1）。
2. その値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う（design-01 の 2）。
3. 外部情報の鮮度は `domain/git_host` の `CacheTtl` を唯一の規則とし、取り直しの間隔を別に持たない（design-01 の 3）。
4. worktree と session から引き当てた Node の id、client が渡した任意のパスから解決した repository のルート、daemon の起動ディレクトリから解決した repository のルートは、いずれも購読の対象にする（design-01 の 4）。
5. agent session の履歴は、購読の対象を「表示している件数」で定義する。件数を増やすときは購読を張り直す（design-01 の 5）。

## 変えないもの

design-01 と design-02 の「変えないもの」をそのまま維持する。今周で人間が新しく指定した維持の条件は無い。

- Workspaces 行の Refresh ボタンと、一覧の保持・失敗した範囲の表示・初回の取得の失敗と「項目なし」の区別。
- 旧 Push の枠組み（`SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`）と監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）。
- Review と Automation が要求するファイル監視（`start_watching`）。
- Settings の他の設定の読み取り（`get_releash_base`、`get_external_editor`、`get_workflow_config` など）。
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-007 と B-009 の「client は監視の開始と停止を要求しない」を、Workspaces の表示のための監視に限ると解釈して Requirements と Behavior を修正した。詳細は `requirements.md` の Assumptions を参照する。今周で新たに自動判断した箇所は無い。
- `[DEFERRED]` で人間へ渡した件: 無し。
- 未決のまま残した要求: 無し。
