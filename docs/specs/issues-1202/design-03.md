# Design 03

## 開始状態

- 差分の基準は `main` の `b05280f9`（`feat/issues/1202` の派生点）。Design 01・Design 02 の周の実装は、未コミットの変更として作業ツリーにある状態を開始状態とする。
- 直前の Design は `docs/specs/issues-1202/design-02.md`。
- この周までに解消となった Thread: 4f809c2d-bfee-458b-ac25-e3fcc3d42ec0（performance build の CLI の既定 data_dir）、d620d048-03ff-47a3-8355-58e94dd7d866（Restart の意図による daemon の終了）ほか Design 02 の周の Thread。見送りとなった Thread はない。
- open Thread 9 件はすべて `[FIX_POLICY]` 付きで今周の変える部分に含める。4b183cf0-ec55-41a7-9d44-fb13b24fb90d は Design 02 の変える部分のうち、`AddRepoPath` → `RepoPathsChanged` の配線検証だけが解消した状態（`[STILL_OPEN]`）である。

## 変える部分

- 本番配線を通る push の検証の残り: `daemon::compose` による本番の組み立てを通った daemon で、agent session、repository state、workflow、review watcher、file watcher、comment 変更の各通知元からの push がクライアント ws で受信されることを検証し、いずれか一つの通知元を ws 配信側と別の sink に配線した場合にも失敗するようにする（現行の `daemon_smoke.rs` は `AddRepoPath` → `RepoPathsChanged` だけを検証する）。根拠: R-004「backend 状態の push ... は、クライアント ws だけで行われる」、B-006、固定ルート1（Design 01）、Thread 4b183cf0-ec55-41a7-9d44-fb13b24fb90d。ルート: 委任
- daemon の local event store 起動拒否の local log 記録: daemon の起動時に local event store を開けず起動を拒否した場合（writer lock の競合等）、その起動拒否を初期化済みの daemon の local log に記録し、stderr を保持しない環境でも local log から確認できるようにする（現行は `lib.rs` の `eprintln!` だけで、変更前は `log::error!("application startup admission failed")` で記録していた）。根拠: R-003「変更前の desktop で利用できた機能……が、変更前と同じ結果で動作する」、R-005、Thread 657c0776-7a5d-4300-9970-ec3e75b5683e。ルート: 委任
- desktop shell の設定の読取元の一本化: desktop shell が適用する設定（ウィンドウの起動・閉じる操作の設定、telemetry の設定）を、`releash.toml` を直接読む別の読取元（`desktop.rs` の `read_config_if_exists`）ではなく daemon が所有する設定状態に従わせ、起動後に設定を変更・ファイルを編集した場合でも shell の適用結果とクライアント ws で取得した設定が一致するようにする。次の 2 項目の結果を満たしたまま行う。根拠: AGENTS.md「状態の所有者を明確にする」「同じ backend-owned state を読める形にする」、R-003、R-004「desktop と daemon の間の req/resp……は、クライアント ws だけで行われる」、Thread 99f73584-75c6-4ee4-b77e-f470a71f8796。ルート: 委任
- 設定の読取失敗時のウィンドウを閉じる操作: desktop 起動後に `releash.toml` が削除・破損・読取不能になっても、normal window を閉じる操作で `prevent_close` のまま無反応にならず、ウィンドウが非表示または最小化されるようにする（現行の `window_lifecycle.rs` は読取失敗でログ出力して return する）。保存済み設定が読める場合に close_to_tray に従う結果（Design 02）は維持する。根拠: R-003、B-005（ウィンドウを閉じる操作）、Thread 5394c619-cce7-46ef-82af-1e73bd598194。ルート: 委任
- クラッシュレポート設定の desktop プロセスへの反映: 設定画面でクラッシュレポートを無効化または有効化した後、再起動を待たずに desktop プロセスと daemon プロセスの双方の Rust panic の送信が新しい設定に従うようにする（現行は daemon の `CRASH_REPORTING_ENABLED` だけを更新する）。根拠: R-003、B-004「設定……の操作を行う THEN 変更前と同じ結果が画面へ反映される」、Design 02「desktop の telemetry 初期化」、Thread 17128476-ece4-4118-910b-0354ce5d778f。ルート: 委任
- daemon の起動時の CLI symlink 設置の検証: daemon の起動経路が CLI symlink の設置を行うこと、そのリンク先が Tauri を使わない実行ファイル（`releash-backend`）であること、リンク経由の実行が Releash の CLI として動作することを検証するテストを置き、起動時の設置呼出しの欠落やリンク先の取り違えで失敗するようにする。根拠: R-009「release build の Releash の起動時に、変更前と同じ条件で `/usr/local/bin/releash` が設置され、`/usr/local/bin/releash` から Releash の CLI を実行できる」、B-011、固定ルート3（Design 01）、`docs/architecture/TEST.md`、Thread 198b67fb-774a-4264-8c1c-ca38ba90ccbc。ルート: 委任
- performance daemon helper の自己テストの実行経路: `tests/helpers/performance-daemon.test.mjs` を、`releash-backend` の準備（performance feature を含むビルド）を伴う経路と対応する runner で実行されるよう登録し、backend をビルドしない `pnpm test:integration`（Playwright）の収集対象から外す。根拠: R-003、R-012「desktop は daemon を起動しない」、Design 02「performance ハーネスの外部 daemon 対応」、Thread 898acf5e-73f7-41a0-96d4-f460054f8013。ルート: 委任
- daemon の起動失敗のエラー生成の簡素化: daemon の起動失敗のエラー生成を、使用されない終了 port・failed authority・`unreachable!` 分岐を経由せずに行い、それのためだけに存在する `DaemonProcessActionPort` の `ProcessLocalExitPort` 実装をなくす（現行の `daemon.rs` の `open_local_event_store` の `map_err`）。起動失敗時の出力内容（safe_description と correlation_id）と終了コードは変えない。根拠: 固定ルート1（Design 01）「依存は明示的に受け取る」、AGENTS.md レビュー観点「同じ概念が二つの場所で表現されていないか」、Thread 9590fae0-38b1-4581-b7ab-bbc7d50062a6。ルート: 委任
- `PathAliases` の本番未使用の既定 data_dir 分岐の削除: `PathAliases::from_runtime` の data_dir 未指定時に `BuildProfile::current()` で既定 data_dir を選ぶ分岐とそのテストをなくし、data_dir の既定値の選択を共通の解決処理だけで行う。子プロセスの alias と data_dir の結果は変えない。根拠: R-008、R-010、固定ルート2（Design 01）、Thread c0b261f8-7869-46c3-b618-129b8d57be20。ルート: 委任

## 固定するルート

- 固定する実装上の指定なし。
- Design 01 の固定ルート1（`AppHandle` の明示依存化と composition root の単一化）、固定ルート2（data_dir 解決の単一化。performance build を含む）、固定ルート3（実行ファイル構成）を今周も維持する。

## 変えないもの

- desktop を daemon なしで起動したときの対処（daemon の起動、接続完了までの待機、失敗理由の表示）を加えない。理由: #1202 は #1203 と同時にリリースする前提であり、#1202 の merge から #1203 の完了までの中間状態は利用者に届かないため（人間の明示。Design 01 から維持）。

## 未確定・リスク

- この周までに自動判断した箇所: なし。
- 自動判断で未決のまま残した要求: なし。
- `[DEFERRED]` で人間へ渡した件: なし。
