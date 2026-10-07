# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1902 （[05] CLI を薄いクライアント crate `releash` にし、CLI と hook を Connect に載せて HTTP を削除する）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、#1901（[04] CLI と hook が使う入口。`docs/specs/issues-1901/` が正）、#1908（[02] `daemon` ドメイン）、#1776・#1777・#1771・#1775、#1877（hook の健全性の監視の削除）、Connect error codes（https://connectrpc.com/docs/protocol#error-codes ）
- 本文の「今の作り」の file:line は main `4533af2f` 時点のもの。この文書は main `5f24cff2`（#1901 の後）で読み直した事実に基づく。
- #1901 で、CLI と hook が使う Connect の入口は揃っている: 購読の対象 `workflow-execution:<id>`、`workflow-output:<id>:<node>`、`review-session-threads:<session_id>[:file=…][:state=…][:author=…][:unread=…][:thread=…]`、`review-worktree-threads:<path>[:file=…][:state=…][:thread=…]`、`review-session-thread:<session_id>:<thread_id>`、`review-session-thread-history:<session_id>:<thread_id>`。RPC `WorkflowSubmitOutput`（worktree 不要）、`DiagnoseWorkflowDirectory`、`CreateSessionReviewThread`・`AppendSessionReviewComment`・`ResolveSessionReviewThread`、`ReceiveProviderSignal`（hook の scope）、`GetServerInfo`（operator と hook の scope。`protocol`・`release`・`serving_status` などを返す）。「無い」は失敗ではなく状態として配信される。
- operator の token は発見ファイル `client-api.json` の token、hook の token はサーバが agent の起動の env `RELEASH_PROVIDER_LIFECYCLE_TOKEN` で渡す（#1901）。
- AGENTS.md「サーバがロジックを所有する」: client に許すのは表示、入力の受付、サーバの呼び出しと購読、受け取ったデータの表示用フォーマットだけ。「通信の原則」: 読み取りは購読、単発の呼び出しは状態を変える操作と入力に対する計算だけ。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。改名（`releashd`、#1903）、画面の監督の削除（#1904）、CLI の status など（#1905）は後続。
- builtin の workflow と利用者の Lua は CLI の `--json` の形に依存している（`workflows/02_implement-existing-spec.yml:170-172` の `.artifact.tasks`、`workflows/03_full-review.yml:312`・`04_review-fix-policy.yml:79`・`04_review-fix-policy-manual.yml:79`・`05_review-fix.yml:176` の review list の `.id`、`workflows/06_handle-pr-review.yml:191` ほかの `output get`）。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-e7 から受けた。

# Outcome

- 対象者: CLI を使う人と agent（`releash workflow …`／`releash review …`）、workflow の Command Node、provider の hook（`releash hook receive`）、Releash の開発者。
- 現在の問題: CLI はサーバと同じ実行ファイルの中にあり、HTTP `/v1` と、サーバを通らずに記録を読み書きする経路（status・output get の直読み、review の直接の読み書き）を持つ。同じ usecase に HTTP と Connect の 2 つの入口があり、HTTP 側は master token で受付・同時実行の枠・scope の外にある。終了コードと出力のエラー表現が独自で、版の違うクライアントとサーバの組を判定できない。
- 変更後の状態: CLI はサーバに依存しない別の実行ファイル `releash` で、全てのコマンドと hook が Connect でサーバを呼ぶ。HTTP `/v1` と master token は無く、サーバを通らずに記録を読み書きする経路は無い。各コマンドは実行ごとに互換を判定し、エラーは Connect の標準コードで表す。

# Current Behavior

main `5f24cff2` で読んで確かめた挙動。パスは `src-tauri/src/` 起点。

- CLI は `releash-backend` の中にあり、引数で daemon（`--internal-daemon`）と背景の作業用プロセス（`--internal-background-worker`）にもなる（`bin/backend.rs:1-11`）。CLI のコードは `cli/`。
- `releash` で届く経路
  - `{data_dir}/bin/releash`（dev は `releash-dev`）の wrapper は daemon 自身の実行ファイルを指す（`infrastructure/platform/path_aliases.rs:79-95`、`:301-317`）。
  - `/usr/local/bin/releash` の symlink は Tauri シェルの隣の `releash-backend` を指す（`releash-desktop/src/infrastructure/platform/cli_install.rs:40-43`）。
  - `.app` には `releash-backend` だけを同梱する（`scripts/build-desktop-backend.mjs:7-23`、`releash-desktop/tauri.conf.bundle.json:7-8`）。dev は `beforeDevCommand` で `releash-backend` だけを作る（`releash-desktop/tauri.conf.json:7`）。
- `workflow status`・`output get` は HTTP `/v1`（`cli/api_client.rs:48-99`）で、発見ファイル `local-api.json` が無い・接続できないときは同じプロセスの中で event store を直接読む（`cli/api_client.rs:126-146`、`cli/file_direct.rs:13-39`）。発見ファイルが残っていて指すプロセスが居ないときは直読みせず終了コード 1（#1777）。store が無い data dir では終了コード 1（#1776）。
- `workflow diagnostics`・`output submit` は HTTP `/v1`（`cli/api_client.rs:58-77`、`:101-110`）。サーバに届かないと「この操作には Releash アプリの起動が必要です」で 1（`:167-188`）。diagnostics は `--dir` の存在を CLI が検査して 4（`cli/diagnostics.rs:49-57`）、error の診断があれば 3（`:72-82`）。
- `review` の全コマンドは HTTP を使わず、同じプロセスで usecase を組み立てて data dir のファイルを直接読み書きする（`cli/review.rs:409-519`）。Session の解決も直接読む（`:212-232`）。値（state／author／unread）は CLI が解釈する（`:241-272`）。`history` の人向け表示は Rust の `Debug` の出力（`:403`）。
- `hook receive` は payload を CLI のプロセスで解釈し（`cli/hook.rs:58-66`）、HTTP POST で送る（`:121-133`）。発見ファイルの master token を使う。失敗を health ファイルに書き（`:155-173`）、SessionStarted の成功で消す（`:136-146`）。常に stdout に `{}` を出して exit 0（`:28-33`）。
- 終了コードは 0／1（その他）／2（入力の誤り）／3（診断が error を検出）／4（見つからない）（`cli/common.rs:3-5`、`:50-56`）。エラーは stderr のテキストで、4 はメッセージだけ、1・2 は `error: <message>`（`:58-63`）。`--json` でもエラーはテキスト。補完と `--data-dir` は無い（`cli/mod.rs:30-35`）。
- `render_long_help()`（`cli/mod.rs:71-104`）を呼ぶのは `cli/mod_test.rs` だけ（#1771）。
- HTTP `/v1`: workflow の 13 ルート（`adaptor/controller/api/workflow.rs:64-105`）と provider lifecycle（`api/provider_lifecycle.rs:28-35`）を master token の `require_bearer`（`api/auth.rs:12-37`。WebSocket の `Sec-WebSocket-Protocol` も受ける `:24`）で認証し、`local_ingress`（`api/mod.rs:65-114`）で同時実行の枠に掛ける。サーバは master token を `local-api.json`、operator の token を `client-api.json` に書く（`infrastructure/local_api/server.rs:56-80`）。
- 画面側の接続先の読み取り（`adaptor/gateway/local_api.rs:186-224` の `ClientConnectionFileQuery`）は 2 つの発見ファイルが同じ instance で token が異なることを検査し、Tauri シェルは `local-api.json` の pid を読む（`releash-desktop/src/adaptor/gateway/daemon_supervision.rs:337-358`）。
- HTTP `/v1` を通しているテスト: `workflow_control_plane_acceptance.rs:716-901`（start、status、log、submit、approve、retry、abort、resume）、`provider_lifecycle_acceptance.rs:506`、`workflow_diagnostics_acceptance.rs:314`、実バイナリ `releash-backend` を起動する `tests/workflow_diagnostics_cli_test.rs:3`・`tests/local_log_cli_regression_test.rs:3`。
- クライアント側の共有 crate は無い。proto の生成物（prost の型、descriptor、Connect の client と server）は `releash-backend` の `build.rs` が作り（`adaptor/presenter/client/mod.rs:37-38`、`adaptor/presenter/connect_wire.rs:3`）、Tauri シェルは `desktop_api`（`desktop_api.rs`）の再公開を通して使う。
- CI は crate ごとの job の組（`frontend-*`／`server-*`／`shell-*` と集約の `frontend`・`server`・`shell`、`quality`。`.github/workflows/ci.yml:26-474`）。

# Scope / Non-goals

## Scope

- クライアント側の共有 crate `releash-client` を作り、proto から生成するクライアント側のもの（prost の型、descriptor、Connect のクライアント）、発見ファイルの読み取りと同一性の確認、互換の判定（`Compatibility`）を置く。`releash-backend` はこの crate の型を使う。
- CLI の crate `releash`（実行ファイル `releash`）を作り、`releash-client` にだけ依存させる。全コマンドと hook を Connect に移す。
- `releash` で届く 3 つの経路（wrapper、`/usr/local/bin/releash`、`.app` の同梱と dev のビルド）を新しい CLI に向ける。
- HTTP `/v1`（workflow と provider lifecycle の router、`require_bearer`、WebSocket subprotocol の対応、`local_ingress`）、master token と `local-api.json`、CLI のクライアント部分（`cli/`、`adaptor/gateway/local_api.rs` の HTTP クライアント、`infrastructure/local_api/client.rs` の HTTP クライアント）、`cli/file_direct.rs`、review のプロセス内組み立て、`render_long_help()` とそのテスト、CLI の `--json` のためだけにある serde の型を削除する。
- 発見ファイルを `client-api.json` 1 つにし、画面側の接続先の読み取りと Tauri シェルの pid の読み取りを追従させる。
- 出力・終了コード・エラーの形を揃え、`releash completion <shell>` と `--data-dir` を足す。
- hook の health ファイルの書き込みと消去をやめる。
- HTTP `/v1` を通しているテストを Connect 経由に移す。
- CI に CLI と共有 crate の job の組を足す。AGENTS.md、`docs/guide/cli.md`（と `docs/guide/workflow/` の該当箇所）を事実に合わせる。
- `docs/architecture/` のうち HTTP に触れている箇所（CONTROLLER.md、PRESENTER.md、README.md、TEST.md）の更新と、TEST.md の配置の表への CLI と共有 crate の行の追加（表の行は利用者の了承が出てから含める）。

## Non-goals

- 改名（`releashd`、#1903）。サーバの実行ファイルは `releash-backend` のまま。
- 画面の監督の削除、Tauri シェルを `releash-client` へ直接切り替えること（シェルは `desktop_api` の再公開のまま動く）。
- CLI の `status`・`server start/stop` など（#1905）、CLI の配置の規則の domain 化（#1907）、ディレクトリ構成の見直し（#1854）。
- hook の健全性の監視のサーバ側（読む側、env `RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE` の受け渡し、画面の表示）の削除（#1877）。ただし、本番の呼び出し元が無くなる unavailable の報告の経路は除く（この ISSUE で消す）。
- review の `--author`／`--unread` の判定単位（#1775）の変更。
- Connect の入口（購読の対象・RPC）の追加と変更。
- main の ruleset の必須のチェックに新しい集約 job を足すこと（merge 前に利用者が GitHub の設定で行う前提）。

# Requirements

- R-001: `releash` は、サーバの実行ファイルとは別の実行ファイルとして提供される。CLI の crate はサーバの crate に依存しない。
- R-002: `{data_dir}/bin/releash`（dev は `releash-dev`）、`/usr/local/bin/releash`、`.app` に同梱された CLI は、どれも新しい `releash` を実行する。`.app` は CLI とサーバの両方を含む。
- R-003: 各コマンドと hook は、実行ごとに発見ファイルからサーバを見つけ、`GetServerInfo` で互換を判定してから呼び出す。状態は持たない。
- R-004: 互換の判定は、クライアントの protocol（proto package のメジャー版）とサーバが返す protocol を比べ、一致すれば互換、一致しなければ大小で「サーバが古い」「クライアントが古い」とする。release は判定に使わない。互換でないとき、コマンドは呼び出しをせず、code `failed_precondition` で、どちらが古いかと双方の release をメッセージに出して失敗する。
- R-005: 発見ファイルが無い・古い（指すプロセスが居ない、同一性が合わない）・壊れている、またはサーバに接続できないとき、全てのコマンドは code `unavailable` で失敗する。メッセージは理由を区別してよい。サーバを通らずに記録を読み書きすることはない。
- R-006: `workflow status`・`workflow output get`・`review list`・`review get`・`review history` は、サーバの状態を購読して最初の状態を受け取り、表示する。実行・Session・Thread が無いときは code `not_found` で失敗する。
- R-007: `workflow output submit`・`review create`・`review comment`・`review resolve` はサーバの変更の呼び出しを、`workflow diagnostics` は診断の呼び出しを 1 回行う。review の書き手と worktree の解決はサーバが行う。
- R-008: `review list` は `--session-id` があればその Session の形で、無ければ `RELEASH_WORKTREE_PATH` の worktree の path の形で一覧を得る。`--author`・`--unread` は `--session-id` を要る。
- R-009: 引数の値の妥当性（execution ID の形、state・author・unread の値、診断の directory の存在など）はサーバが判定し、CLI は同じ判定を持たない。
- R-010（互換性）: 各コマンドの `--json` の stdout は、キー名と値が今と同じである。
- R-011: 既定の出力は人向けの表示で、`--json` のときは機械向けの JSON である。`review history` の人向け表示はプログラム内部の表現をそのまま出さない。
- R-012: エラーは stderr に出る。`--json` のとき、エラーは stderr に `{"error":{"code":"<code>","message":"<message>"}}` で出る。`code` は Connect の標準コード名（`not_found`、`invalid_argument`、`unavailable` など）である。
- R-013: 終了コードは 0（成功）、1（失敗）、2（引数の構文の誤り）、3（`workflow diagnostics` が severity error の診断を 1 件以上検出した）の 4 値である。サーバから返った失敗は `invalid_argument` を含めて 1 である。4 は使わない。
- R-014: `releash completion <shell>` は、指定した shell の補完スクリプトを stdout に出す。
- R-015: `--data-dir <PATH>` は全コマンドで指定でき、`RELEASH_DATA_DIR` と同じ意味を持ち、両方あれば `--data-dir` が優先される。
- R-016: `hook receive` は、stdin の payload を解釈せずに、env の hook の token で `ReceiveProviderSignal` に送る。結果に関わらず exit 0 で、失敗（payload が上限の 65,536 byte を超える、サーバに届かない、互換でない、拒否された など）は stderr に出す。payload が上限を超えるときと互換でないときは送らない。health ファイルを読み書きしない。
- R-017: サーバは HTTP `/v1` を提供せず、master token と `local-api.json` を作らない。サーバが書く発見ファイルは `client-api.json` だけである。
- R-018: Tauri アプリは、発見ファイルが `client-api.json` だけの状態で、今と同じにサーバを起動・監督・接続できる。
- R-019: 本番から呼ばれていない `render_long_help()` は存在しない。
- R-020: CI は、CLI と共有 crate の fmt・clippy・単体テスト（CLI は統合テストも）を、crate ごとの job の組で検査する。
- R-021: AGENTS.md と `docs/guide/cli.md`・`docs/guide/workflow/` は、CLI の構成・終了コード・エラーの形・サーバ未起動時の挙動・HTTP の不在について今のコードと一致する。

# Assumptions

- `docs/architecture/` のうち HTTP に触れている箇所（CONTROLLER.md、PRESENTER.md、README.md、TEST.md:63）の変更は、利用者が了承済み。文面は releash-e7 が決め、Design に書いた。TEST.md の配置の表への CLI と共有 crate の行の追加は、利用者の了承を待つ。了承が出たものだけを含める。
- main の ruleset の必須のチェックへの新しい集約 job 名の追加は、merge の前に利用者が GitHub の設定で行う。
