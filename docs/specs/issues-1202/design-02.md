# Design 02

## 開始状態

- 差分の基準は `main` の `b05280f9`（`feat/issues/1202` の派生点）。Design 01（`docs/specs/issues-1202/design-01.md`）の周の実装は、未コミットの変更として作業ツリーにある状態を開始状態とする。
- 直前の Design は `docs/specs/issues-1202/design-01.md`。
- この周までに解消・見送りとなった Thread はない。open Thread 13 件はすべて `[FIX_POLICY]` 付きで今周の変える部分に含める。

## 変える部分

- performance build の CLI・provider hook の既定 data_dir: `RELEASH_DATA_DIR` が未設定または空のとき、performance build の `releash workflow` / `releash review` / `releash hook` が daemon と同じ `<OS のデータディレクトリ>/com.releash.app.performance` を使うようにする（現行の `cli/common.rs` は `BuildProfile::current()` により `com.releash.app` に解決する）。空でない `RELEASH_DATA_DIR`、release・dev の既定は変えない。根拠: R-008「performance build で `RELEASH_DATA_DIR` が未設定または空のときの既定の data_dir は、daemon と同じ `<OS のデータディレクトリ>/com.releash.app.performance` とする」、B-017、Thread 4f809c2d-bfee-458b-ac25-e3fcc3d42ec0。ルート: 委任（固定ルート2 は維持）
- Restart の意図による daemon の終了: application quit の Restart の意図でも、Exit と同じ一括停止（workflow runtime / terminal surface / provider exit observer / local API）の完了後に、接続や sender の寿命に依存せず daemon のプロセスを終了させる（現行の `DaemonProcessActionPort` は Restart でログ出力のみ）。再起動処理は追加せず、Exit の挙動は変えない。根拠: R-015「終了（Exit）または再起動（Restart）の意図を受けると……その完了後に daemon のプロセスが終了する」、B-016、Thread d620d048-03ff-47a3-8355-58e94dd7d866。ルート: 委任（終了通知の経路と終了コードを含む）
- 停止済み daemon の discovery file の拒否: desktop が discovery file を読んで接続先を決めるとき、記載の pid・開始時刻に一致する生存プロセスがなければその接続先を受理せず、client token の送信・接続・command 送信を行わない（現行の `ClientConnectionFileQuery::read` はファイル間の一致だけで受理する）。起動済みの daemon の discovery file では従来どおり接続する。根拠: R-012「起動済みの daemon へクライアント ws で接続する」、B-014、R-006、B-008、Thread 7147f606-7863-4731-9209-4ee87f3ffdf2。ルート: 委任
- ウィンドウを閉じる操作の設定の Rust 所有: 保存済みの close_to_tray が false なら desktop 起動後の最初の CloseRequested から minimize、true なら tray へ非表示とし、結果が renderer の設定中継の完了・タイミング・失敗に依存しないようにする（現行は `WindowPreferencesState` を既定 None で登録し、None を close_to_tray=true として扱う）。根拠: R-003「変更前の desktop で利用できた機能（UI shell に残す物を含む）が、変更前と同じ結果で動作する」、B-005（ウィンドウを閉じる操作）、Thread e87b723a-27de-41ae-9874-ff91922eef58。ルート: 委任
- desktop の telemetry 初期化: 変更前の設定に従い desktop プロセス自身で telemetry と panic hook を初期化し、desktop の Rust panic の送信と `startup.app` / `startup.first_window_ready` の記録を行う（現行は daemon 側だけで初期化している）。daemon 側の観測は変えない。根拠: R-003、`docs/specs/issues-1209/requirements.md` 要求10・13、Thread c72a14a7-6b2f-44c8-8055-192c7f4846cb。ルート: 委任
- performance ハーネスの外部 daemon 対応: clean な performance 保存先から従来の performance スクリプト（`package.json` の `test:performance:tauri` / `test:performance:launch:*`、`wdio.performance.conf.ts`）を実行すると、同じビルド種別の daemon が fixture 用の環境（`RELEASH_PERFORMANCE_LAUNCH_PROVIDER` 等）を受け取って起動済みになり、desktop がその daemon に接続して setup が成功し、実 Tauri の terminal／agent 計測を開始できるようにする。desktop が daemon を起動しないことは維持する。根拠: R-003、R-005、R-012「desktop は daemon を起動しない」、Thread aaa3a4a3-454c-4e46-9f1a-c99e091dfaa9。ルート: 委任
- daemon 起動時の PATH 書換えの順序: daemon の起動で、process-global な PATH の書換えを log writer thread や Tokio worker など他 thread の起動後に行わない（または process-global な書換えに依存しない伝搬にする）。provider 実行ファイルの探索と子プロセスの PATH は変更前と同じ結果にする（現行は `local_log::init` → `Runtime::new` → `compose` 内の `set_var`）。根拠: R-003、R-010、Rust 公式 `std::env::set_var` の Safety、Thread 762a59e6-0060-46cc-813c-27091ee15fc2。ルート: 委任
- presenter の infrastructure 依存の解消: `adaptor/presenter/agent_session_changed.rs` を含む presenter が `infrastructure::push::PushSink` など infrastructure の型に依存せず、push の送信機構への依存を gateway 側に閉じる。agent session の変更 push は変更前と同じくクライアント ws で配信する。根拠: R-004、B-006、`docs/architecture/README.md` の依存方向、`docs/architecture/GATEWAY.md`「外向き通知の送信」、Thread 166f2eaf-2b6e-4018-8c25-3f149f139b5e。ルート: 委任
- `WorkflowRuntimeDependencies` の不要な data_dir の削除: 使用されない `data_dir` フィールドと、workflow 開始経路で data_dir を取り出して捨てる処理をなくし、workflow host が使う data_dir の配線を一つにする。workflow 開始の挙動は変えない。根拠: 固定ルート1「依存は明示的に受け取る」（Design 01）、Thread 2e07ad7d-bcbc-41b9-82a2-34a96835af7e。ルート: 委任
- desktop feature 無効時の gateway 単体テストの構築: `--no-default-features` で library の cfg(test) を含めてビルドでき、daemon 所有の workflow_host・terminal_surface/runtime_gateway_impl の gateway 単体テストが desktop 限定モジュール（`desktop_test_support`）や Tauri の managed state に依存せず構築・実行できるようにする。根拠: R-001、B-002、固定ルート3（Design 01）、`docs/architecture/TEST.md`「`adaptor/gateway/` 必須」、Thread baa300fb-62f5-4bc3-899b-9b011e702e94。ルート: 委任
- 本番配線を通る push の検証: `daemon::compose` による本番の組み立てを通った daemon で状態変更を起こし、その push がクライアント ws で受信されることを検証するテストを置き、通知元と ws 配信側が別の sink に配線された場合に失敗するようにする。根拠: R-004、B-006、固定ルート1（Design 01）、Thread 4b183cf0-ec55-41a7-9d44-fb13b24fb90d。ルート: 委任
- daemon 再起動テストの port 条件の除去: `tests/desktop_daemon.rs` と `tests/helpers/desktop-daemon.mjs` が port・URL の不一致を必須条件にせず、instance_id・token 等で別インスタンスへの再接続を検証し、OS が旧 port を再割当てした場合でも正しい再接続実装で成功するようにする。根拠: R-011「desktop は新しいインスタンスへ再接続し」、B-013、Thread 36cb75c0-09da-4558-ae2f-2cc8ba357d6c。ルート: 委任
- 結合テストの実環境からの隔離: 実 daemon を起動する結合テスト（`tests/desktop_daemon.rs`、`tests/daemon_smoke.rs`）を macOS で実行しても、利用者の `~/Library/Application Support/releash/workflows` など実環境の設定・生成物（`.releash` 配下、`.luarc.json`）を作成・上書きせず、テストが扱う設定・生成物を一時領域に限定する。production の保存先仕様は変えない。根拠: R-005、B-007、`docs/architecture/TEST.md`、Thread c6d53b34-e98c-44c1-bfd8-a928c7ccddf1。ルート: 委任

## 固定するルート

- 固定する実装上の指定なし。
- Design 01 の固定ルート1（`AppHandle` の明示依存化と composition root の単一化）、固定ルート2（data_dir 解決の単一化。performance build を含む）、固定ルート3（実行ファイル構成）を今周も維持する。

## 変えないもの

- desktop を daemon なしで起動したときの対処（daemon の起動、接続完了までの待機、失敗理由の表示）を加えない。理由: #1202 は #1203 と同時にリリースする前提であり、#1202 の merge から #1203 の完了までの中間状態は利用者に届かないため（人間の明示。Design 01 から維持）。

## 未確定・リスク

- この周までに自動判断した箇所: なし。
- 自動判断で未決のまま残した要求: なし。
- `[DEFERRED]` で人間へ渡した件: なし。
