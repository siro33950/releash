# Design 01

## 開始状態

- 初回。差分の基準は `main` の `b05280f9`（`feat/issues/1202` の派生点。PR #1824 merge 後）で、未コミットの変更は `docs/specs/issues-1202/` の文書だけである。
- 実装の状態は `requirements.md` の Current Behavior を参照する。
- 解消・見送りとなった Thread はない。

## 変える部分

- 実行ファイルの 2 分割: 現行の 1 つの実行ファイル `releash`（`main.rs` の GUI / CLI 分岐）を、desktop shell（Tauri、GUI のみ）と、Tauri を使わない実行ファイル（daemon モード＋CLI の `workflow` / `review` / `hook`）に分ける。根拠: R-001「daemon の実行ファイルは Tauri に依存しない」、B-002、R-008、B-010。ルート: 固定ルート3
- backend の組み立てと依存の明示化: `lib.rs` の Tauri `setup` 内で行っている backend の組み立て（`app.manage` を含む）を daemon の composition root 一か所へ移し、`AppHandle` から managed state や data_dir を取得している daemon 側の箇所（`BackendPush::emit` の `PushSink` 取得、workflow_host とその submodule、runtime_command_gateway、terminal_surface/runtime_gateway_impl、secret_source、event_log_writer など）を明示的に受け取る依存へ置き換える。根拠: R-001、B-002、R-004「backend 状態の push は Tauri event で配信されない」、B-006。ルート: 固定ルート1
- daemon の headless 単独起動: daemon モードで起動すると、ウィンドウ・メニュー・トレイアイコンを表示せずに、data_dir の local event store を開き、local API とクライアント ws を 127.0.0.1 に bind し、`<data_dir>/local-api.json` を書き出し、クライアント token で認証した接続のエンベロープの command を実行して同じ `request_id` で応答する。根拠: R-001、B-001、R-002「daemon を desktop なしで単独起動すると……同じ `request_id` の応答を返す」、B-003、R-007、B-009。ルート: 委任
- daemon の単独起動手段の非公開: daemon モードの起動手段を CLI のヘルプと `docs/guide/` に現れない形にする。根拠: R-013「利用者向けの入口として提供されない」、B-015。ルート: 固定ルート3（daemon モードは Tauri を使わない実行ファイルに含める）。起動引数・起動手段の形は委任
- data_dir 解決の単一化: data_dir を Tauri を使わない 1 つの処理で決め、daemon・desktop shell・CLI がそれを使う。`infrastructure/platform/app_data_dir.rs` の `app.path().app_data_dir()` による解決は削除する。release / dev / performance で変更前の desktop と同じ場所を指す。根拠: R-005、B-007、R-008、R-010、R-012、B-014。ルート: 固定ルート2
- desktop shell の外部 daemon への接続: desktop shell は daemon を起動せず、同じビルド種別の data_dir にある discovery file を手がかりに、起動済みの daemon へクライアント ws で接続する。renderer にはクライアント ws の接続情報とクライアント token だけを渡し、master token を渡さない。根拠: R-012「desktop は daemon を起動しない」、B-014、R-006、B-008。ルート: desktop shell が接続情報とクライアント token を取得する方法は委任
- daemon の別インスタンスへの再接続: desktop の接続先の daemon が停止して別のインスタンスとして起動し直された場合に、desktop が新しいインスタンスの接続情報で再接続し、#1201 の結果不明・再実行抑止の規則のまま動作するようにする。根拠: R-011、B-013。ルート: 委任
- renderer の起動結果取得: renderer が成功時にも Tauri invoke の `get_application_startup_outcome` で起動結果を取得している経路を、backend が別プロセスになった desktop shell で成立させる。根拠: R-003、B-004、B-014。ルート: 委任
- UI shell に残す物の desktop shell への配置: dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、`set_menu_items_enabled`、menu / tray / window lifecycle を desktop shell で提供し、変更前と同じ結果にする。根拠: R-003、B-005。ルート: 委任
- 外部エディタ起動の Tauri 非依存化: `adaptor/gateway/external_editor/launcher_impl.rs` の `tauri_plugin_opener` による起動を、Tauri を使わない方法に置き換え、変更前と同じ結果にする。根拠: R-001、B-002、R-003、B-004（外部エディタの操作）。ルート: 委任
- Exit の意図による daemon の終了: application quit の Exit の意図で、`ShutdownCoordinator` の一括停止（workflow runtime / terminal surface / provider exit observer / local API）の完了後に、Tauri の `exit` ではなく daemon のプロセスを終了させる。根拠: R-015、B-016。ルート: 委任
- `/usr/local/bin/releash` の設置先: release build の起動時に、変更前と同じ条件で `/usr/local/bin/releash` を Tauri を使わない実行ファイルへの symlink として設置する。根拠: R-009、B-011。ルート: 固定ルート3（Tauri を使わない実行ファイルを指す）。設置主体は委任
- 子プロセスの CLI alias と provider hook の受け口: terminal・agent session の子プロセスの alias wrapper（`<data_dir>/bin/releash` / `releash-dev`）と、それを経由する provider hook の `releash hook receive` が、Tauri を使わない実行ファイルを呼び、daemon と同じ data_dir を `RELEASH_DATA_DIR` で渡すようにする。根拠: R-010、B-012、R-008、B-010。ルート: 固定ルート3

## 固定するルート

- 固定ルート1（`AppHandle` の明示依存化と composition root の単一化）
  - 範囲: daemon 側の全体。backend 状態の push の `BackendPush`、workflow_host とその submodule、runtime_command_gateway、terminal_surface/runtime_gateway_impl、secret_source、event_log_writer など、現在 `AppHandle` から managed state や data_dir を取得している箇所すべて。
  - 粒度: 依存は明示的に受け取る。`AppHandle` を Tauri 以外の型マップ・グローバル等のサービスロケータに置き換えない。組み立ては daemon の composition root 一か所に集約する。
  - 理由: Issue #1202 が、`AppHandle` のサービスロケータ化と、composition root が単一でないこと（`AppState` が 9 usecase のみ、`lib.rs` の `app.manage` 多数）を問題として挙げているため。
- 固定ルート2（data_dir 解決の単一化）
  - 範囲: daemon・desktop shell・CLI の data_dir 解決。
  - 粒度: data_dir は Tauri を使わない 1 つの処理で決め、三者がそれを使う。Tauri の `app.path().app_data_dir()` による解決（`infrastructure/platform/app_data_dir.rs`）は desktop shell にも残さない。release / dev / performance の 3 ビルド種別すべてで変更前と同じ場所を指す。Issue #1202 の「app_data_dir は desktop shell 側に残す」は、この項目についてのみ採用しない。
  - 理由: 同じ data_dir に解決方法が 2 つ並ぶと、desktop と daemon が出会う discovery file の場所がずれ、接続できなくなるため。
- 固定ルート3（実行ファイル構成）
  - 範囲: 実行ファイルの構成と CLI の受け口。
  - 粒度: 実行ファイルを 2 つにする。(1) desktop shell（Tauri、GUI のみ）、(2) Tauri を使わない実行ファイル（daemon モード＋CLI の `workflow` / `review` / `hook`）。`/usr/local/bin/releash`、terminal・agent session の子プロセスの alias wrapper（`<data_dir>/bin/releash` / `releash-dev`）、provider hook の `releash hook receive` は (2) を指す。daemon モードは (2) に含まれるため、R-013 に従い CLI のヘルプに現れない。symlink の設置主体は委任。
  - 理由: Issue の目的「Tauri 非依存の headless 単独バイナリ」と R-001 を満たし、すでに Tauri 非依存の CLI と daemon を 1 つにまとめることで、alias wrapper の「自身の実行ファイルを呼ぶ」仕組みのまま CLI と hook に到達できるため。現行 1 実行ファイルに daemon モードを足す案は R-001 / B-002 に反するため採用しない。3 分割は理由がないため採用しない。

## 変えないもの

- desktop を daemon なしで起動したときの対処（daemon の起動、接続完了までの待機、失敗理由の表示）を加えない。理由: #1202 は #1203 と同時にリリースする前提であり、#1202 の merge から #1203 の完了までの中間状態は利用者に届かないため（人間の明示）。

## 未確定・リスク

- performance build の CLI の data_dir: 現行の CLI の既定の data_dir は `path_aliases::BuildProfile::current()` が `cfg!(debug_assertions)` だけで決めており、performance build（`tauri build --features performance-wdio --config src-tauri/tauri.conf.performance.json`、release profile）の CLI は `RELEASH_DATA_DIR` がないと `com.releash.app` に解決する。一方、変更前の performance build の desktop は `com.releash.app.performance` を使う。固定ルート2 で CLI も同じ解決処理を使うと、performance build で `RELEASH_DATA_DIR` なしに実行した CLI の解決先が `com.releash.app.performance` に変わり、R-008「変更前と同じ data_dir の解決」と食い違う可能性がある。問い: performance build で `RELEASH_DATA_DIR` なしに実行した CLI の data_dir は、変更前の `com.releash.app` と、daemon と同じ `com.releash.app.performance` のどちらを正とするか。
- この周までに自動判断した箇所: なし。
- 自動判断で未決のまま残した要求: なし。
- `[DEFERRED]` で人間へ渡した件: なし。
