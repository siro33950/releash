# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1885` の `42be41e0`。直前の Design は `docs/specs/issues-1885/design-04.md`（`context.design_path` と一致する）。
- 開始状態の実装は、`42be41e0` に design-01・design-02・design-03・design-04 の変更を適用した未コミットの作業ツリーである（176 件）。これら 4 つの Design の「変える部分」は実装済みであり、この Design では再掲しない。design-04 の 2 件は、`src/components/workspace/WorkspaceList.test.tsx` の hook mock から `refreshWorktree`・`refreshRepository` とそれらの非呼出しの確認が無くなっていること、`src-tauri/src/client_api_acceptance.rs:546-557` の `read_current_branch` が `SubscriptionTarget::from_parts` と `to_string` を経由していることで確認した。
- この周までに解消・見送りとなった Thread: design-04 が扱った 2 件（`bdf0d095`、`96fd3ba4`）はいずれも resolved である。`[DEFERRED]` と `[REJECTED]` は今周 0 件である。
- open Thread は 3 件（`9b2cc991`、`3a0d998f`、`b2e40d2e`）で、いずれも `[FIX_POLICY]` が付いている。`9b2cc991` は resolved の `d63a3739` を再発元に持つ。
- Requirements と Behavior は今周で変更していない。対応表は R-001〜R-016 を漏れなく含み、Behavior は B-001〜B-023 で欠番・重複・参照切れは無い。誤り・不足・矛盾は見つからなかった。

## 変える部分

Requirements・Behavior の変更は無い。以下は `[FIX_POLICY]` が付いた open Thread 3 件に対応する変更である。

- workflow execution の archive / restore の成功後の通知値を worktree path にする: `src-tauri/src/usecase/workflow/execution_archive.rs:121-127` と `:168-174` が `StateChangeSource::Worktree` へ `target.workspace_identity` を渡している。購読側は `src-tauri/src/domain/state_subscription/subscription_target.rs:198-204` が Selection・NodeDetail・SessionNode・SessionHistory を購読引数の worktree path との完全一致で選び、`src-tauri/src/usecase/state_subscription/reads.rs:232-238` も通知値を `refresh_worktree` のパスとして使う。`ExecutionTreeArchiveTarget` は `worktree_path` と `workspace_identity` を別の値として持つ（`src-tauri/src/domain/workflow/repository.rs:27-33`）ため、両者が異なる実行木では archive / restore 後に購読が更新されない。根拠: R-004「Workspaces の表示は、client からの取り直しの呼び出しを行わずに、daemon の状態の変化を反映する」、B-004、Thread `9b2cc991-ed4e-4b1b-a767-364a62215855`。ルート: 委任。
- `beginArchiveReconciliation` の実行されない非同期の契約と使われない戻り値をやめる: `src/hooks/useWorkspaceTreeNodes.ts:47-59` は ref と state を同期に更新するだけで `await` を含まず常に `null` を返し、唯一の本番呼び出し元 `src/components/workspace/WorkspaceList.tsx:905-906` は戻り値を使わない。呼び出し元も同じ契約で呼ぶ。archive 後の突き合わせの外部から観測できる振る舞いは変えない。根拠: R-015「本 ISSUE で使われなくなるコードと、本 ISSUE で触れたファイルの中で使われていないコードは無い」、B-016「この変更で使われなくなったコードは残らない」、Thread `3a0d998f-0b86-4a86-940e-f6e5e7894aeb`。ルート: 委任。
- `requestError` の producer が無いフィールドと、その値に依存する表示の分岐をやめる: `src/hooks/useWorkspaceList.ts:9` の `requestError` が `path?: string` を持つ一方、生成箇所は `:26` の `refresh` の失敗と `:33` の `subscription.error` のいずれも `message` だけであり、リポジトリ内に `path` を設定する箇所が無い。`src/components/workspace/WorkspaceList.tsx:1704` がその値を条件分岐して表示している。Workspaces の更新の失敗時に表示される内容は変えない。根拠: R-015、B-016、Thread `b2e40d2e-7046-4a5b-9314-8f769c73d579`。ルート: 委任。

## 固定するルート

今周に新しく人間が指定したルートは無い（`materials.design.directions` は空）。design-01 の「固定するルート」1〜5 は今周も維持し、解除・変更しない。

1. 購読対象を表す値オブジェクトを `domain/state_subscription` に置き、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵をその型にする（design-01 の 1）。
2. その値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う（design-01 の 2）。
3. 外部情報の鮮度は `domain/git_host` の `CacheTtl` を唯一の規則とし、取り直しの間隔を別に持たない（design-01 の 3）。
4. worktree と session から引き当てた Node の id、client が渡した任意のパスから解決した repository のルート、daemon の起動ディレクトリから解決した repository のルートは、いずれも購読の対象にする（design-01 の 4）。
5. agent session の履歴は、購読の対象を「表示している件数」で定義する。件数を増やすときは購読を張り直す（design-01 の 5）。

## 変えないもの

design-01・design-02・design-03・design-04 の「変えないもの」をそのまま維持する。今周で人間が新しく指定した維持の条件は無い。

- Workspaces 行の Refresh ボタンと、一覧の保持・失敗した範囲の表示・初回の取得の失敗と「項目なし」の区別。
- 旧 Push の枠組み（`SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`）と監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）。
- Review と Automation が要求するファイル監視（`start_watching`）。
- Settings の他の設定の読み取り（`get_releash_base`、`get_external_editor`、`get_workflow_config` など）。
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-007 と B-009 の「client は監視の開始と停止を要求しない」を、Workspaces の表示のための監視に限ると解釈して Requirements と Behavior を修正した。詳細は `requirements.md` の Assumptions を参照する。今周で新たに自動判断した箇所は無い。
- `[DEFERRED]` で人間へ渡した件: 無し。
- 未決のまま残した要求: 無し。
