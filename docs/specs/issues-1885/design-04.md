# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1885` の `42be41e0`。直前の Design は `docs/specs/issues-1885/design-03.md`（`context.design_path` は `design-02.md` だが、`spec_dir` には実装済みの `design-03.md` があり、直前の周はそちらである）。
- 開始状態の実装は、`42be41e0` に design-01・design-02・design-03 の変更を適用した未コミットの作業ツリーである（176 件）。これら 3 つの Design の「変える部分」は実装済みであり、この Design では再掲しない。
- この周までに解消・見送りとなった Thread: design-03 が扱った 5 件のうち 4 件（`e6493169`、`88a01439`、`623a47b2`、`bf4891b6`）は resolved である。残る `96fd3ba4` は、client 側（`src/lib/client.ts:250` の `stateTargetKey`）が `JSON.stringify([kind, args])` へ変わって解消した一方、同じ規則の別の該当箇所が見つかり open のままである。`[DEFERRED]` と `[REJECTED]` は今周 0 件である。
- open Thread は 2 件（`bdf0d095`、`96fd3ba4`）で、いずれも `[FIX_POLICY]` が付いている。
- Requirements と Behavior は今周で変更していない。対応表は R-001〜R-016 を漏れなく含み、Behavior は B-001〜B-023 で欠番・重複・参照切れは無い。誤り・不足・矛盾は見つからなかった。

## 変える部分

Requirements・Behavior の変更は無い。以下は `[FIX_POLICY]` が付いた open Thread 2 件に対応する変更である。

- `WorkspaceList` のテストの hook mock から削除済みの取り直しのフィールドを除く: `src/components/workspace/WorkspaceList.test.tsx:137` の `useWorkspaceTreeNodes` の mock が返す `refresh`、および `:162-163` の `useWorkspaceList` の mock が返す `refreshWorktree`・`refreshRepository` をやめ、`:1832`、`:2372`、`:2379`、`:2445` のそれらの非呼出しの確認も残さない。開始状態では `src/hooks/useWorkspaceTreeNodes.ts:77-92` の戻り値に `refresh` が無く、`src/hooks/useWorkspaceList.ts:7-11` の `WorkspaceListModel` は `snapshot`・`requestError`・`refresh` だけで `refreshWorktree`・`refreshRepository` を持たないため、本番の `WorkspaceList` はこれらを読めず、非呼出しの確認は常に成立する。`:161` の `refresh` は `WorkspaceListModel` に存在するため対象外とする。根拠: R-015「本 ISSUE で使われなくなるコードと、本 ISSUE で触れたファイルの中で使われていないコードは無い」、B-016「この変更で使われなくなったコードは残らない」、Thread `bdf0d095-dbbc-4744-a353-56d691cde4cf`。ルート: 委任。
- 購読対象名のエンコードの所有者の一本化（残る該当箇所）: 対象名の組み立て規則が domain（および対象名との相互変換を担う adaptor）だけに存在し、Rust のテスト補助が同じ規則を自前で組み立てないようにする。開始状態では `src-tauri/src/client_api_acceptance.rs:546-551` の `read_current_branch` が `format!("current-branch:{}:{path}", path.len())` で `{len}:{arg}` の長さ接頭辞規則を手で組み立て、直後の `read_state` が `SubscriptionTarget::parse` で解釈し直している。domain の `from_parts` と `Display` を経由すれば同じ対象名を得られる。design-03 が同じ変更を挙げたが、そこで対象としていた `src/lib/client.ts` の箇所は解消済みであり、この箇所が残っている。根拠: 「固定するルート」1（design-01 の 1、design-02・design-03 で維持）、Thread `96fd3ba4-6afa-49ad-abae-9a2d289fbb34`。ルート: 委任。

## 固定するルート

今周に新しく人間が指定したルートは無い（`materials.design.directions` は空）。design-01 の「固定するルート」1〜5 は今周も維持し、解除・変更しない。

1. 購読対象を表す値オブジェクトを `domain/state_subscription` に置き、対象名の組み立て・解釈・引数の検証をそこが所有する。`Subscriptions` の鍵をその型にする（design-01 の 1）。
2. その値オブジェクトが「必要とする監視（種類とパス）」を持ち、`Subscriptions` が購読者の増減から開始すべき監視・停止すべき監視を導く。監視の実行（I/O）は usecase が行う（design-01 の 2）。
3. 外部情報の鮮度は `domain/git_host` の `CacheTtl` を唯一の規則とし、取り直しの間隔を別に持たない（design-01 の 3）。
4. worktree と session から引き当てた Node の id、client が渡した任意のパスから解決した repository のルート、daemon の起動ディレクトリから解決した repository のルートは、いずれも購読の対象にする（design-01 の 4）。
5. agent session の履歴は、購読の対象を「表示している件数」で定義する。件数を増やすときは購読を張り直す（design-01 の 5）。

## 変えないもの

design-01・design-02・design-03 の「変えないもの」をそのまま維持する。今周で人間が新しく指定した維持の条件は無い。

- Workspaces 行の Refresh ボタンと、一覧の保持・失敗した範囲の表示・初回の取得の失敗と「項目なし」の区別。
- 旧 Push の枠組み（`SubscribePush`、`Push`、`WatchFiles`、`WatchGitDirectory`、`StopWatching`）と監視の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs`）。
- Review と Automation が要求するファイル監視（`start_watching`）。
- Settings の他の設定の読み取り（`get_releash_base`、`get_external_editor`、`get_workflow_config` など）。
- 状態を変える操作の呼び出し方。単発の呼び出しのまま変えない。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-007 と B-009 の「client は監視の開始と停止を要求しない」を、Workspaces の表示のための監視に限ると解釈して Requirements と Behavior を修正した。詳細は `requirements.md` の Assumptions を参照する。今周で新たに自動判断した箇所は無い。
- `[DEFERRED]` で人間へ渡した件: 無し。
- 未決のまま残した要求: 無し。
