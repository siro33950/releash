# Design 03

## 開始状態

- 差分の基準: `main` から派生した `feat/issues/1835`。派生点は `2774f470`（release: v0.4.14）。
- 直前の Design: `docs/specs/issues-1835/design-02.md`。同 Design の周の実装は未コミットの作業ツリーに入っており、このコードを今周の開始状態とする。同 Design のルート6（`log::logger().flush()` を終了処理の直列段階から外し、OTLP shutdown は 15 秒の上限の内側に残す）とルート7（`LocalDomainEvent` の variant 名の差し戻し）は、`usecase/application_lifecycle/mod.rs:13-38`、`domain/local_event/events.rs:14-18` に反映済みである。
- Requirements・Behavior: 今周は変更しない。`requirements.md` の R-001〜R-007、`behavior.md` の B-001〜B-007 と対応表は据え置く。R-001〜R-007 のすべてに Behavior ID が対応しており、誤り・不足・矛盾は確認していない。
- この周までに解消・見送りとなった Thread: design-02 の周で扱った 10 件のうち 9 件が resolved である。40cf5057、fb7b03d4、9827a897、1d9f51b2、2786eb17、4d52752c、876360bf、4a5765cc は修正による解消。3125c385 は `[DEFERRED]` として、打ち切り経路でのログの永続化を要求に加えないという人間の決定を維持したまま解消した。
- Thread: `[FIX_POLICY]` が付いた open Thread が 6 件あり、今周で変える部分はこの 6 件である（57fd56e4、d9896ab7、d5f31c55、9ab5d50a、f60fcd49、eb399c04）。`[REJECTED]`・`[DEFERRED]` と判定された Thread は今周に無い。

## 変える部分

- 終了開始後に届いた終了要求の保持量: 終了処理の開始後に届いた終了要求が、未処理のまま要求数に比例して保持されないようにし、最初の終了要求だけが終了処理を起動するようにする。開始状態では `src-tauri/src/adaptor/gateway/application_lifecycle.rs:8-19` の `DaemonProcessActionPort::execute` が要求ごとに `UnboundedSender` へ send し、`src-tauri/src/adaptor/controller/daemon.rs:14` の `Daemon::wait` が 1 件だけ recv して以後受信しない。`usecase/application_lifecycle/mod.rs:6-11` と `adaptor/controller/client/application_lifecycle.rs` にも後続要求を統合・拒否する判定はない。R-004 が保存しないと定める終了要求の呼び出し記録（永続記録）は、この変更でも保存しない。根拠: Thread 57fd56e4、R-004。ルート: 委任。
- 段階の失敗ログの検証: 終了処理でのターミナルの停止・出力排出・checkpoint 保存のいずれの個別ログが欠けても検証が失敗し、複数の失敗が同時に起きた場合も各失敗がログに現れることを確認できるようにする。開始状態では `src-tauri/src/usecase/terminal_surface/application.rs:558-599` の `shutdown` が 3 分岐で `log::error!` を出して `first_error` だけを返す一方、`application_test.rs:692-740` の検証は `result.is_err()` と gateway の呼び出し順だけで、3 つの `log::error!` を削除しても成立する。根拠: Thread d9896ab7、R-002、B-002。ルート: 委任。
- application lifecycle のテストの所属: `src-tauri/src/adaptor/controller/application_lifecycle_test.rs` の 2 件の検証を、対象実装と同じディレクトリの `<impl>_test.rs` から取り込むようにする。検証内容（終了要求の受理と受信先不在時の失敗）は変えない。開始状態では `controller/application_lifecycle.rs:56-58` が取り込むこの 2 件が、`adaptor/controller/client/application_lifecycle::request_application_quit_shared` と `adaptor/gateway/application_lifecycle::DaemonProcessActionPort` だけを呼び、同ファイルの `ApplicationQuitIngress`・`TauriApplicationQuitIntentPort` を使わない。根拠: Thread d5f31c55、規約 `docs/architecture/TEST.md` の配置節。ルート: 委任。
- `command_admission` の guard の保持範囲: 独立した execution の command の起動が、他の command の登録後の同期・通知・observer 準備の完了を一律に待たないようにする。終了開始後に受理した command が停止対象から漏れないこと（R-001 / B-001 が求める起動と停止の排他）は保つ。開始状態では `src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:1603` で取得する runtime 全体で 1 つの guard を、プロセス登録・`commit_command_spawned`・execution store の同期と broadcast・`command_completion_observers` への登録を終えた `:1722` の drop まで保持する。根拠: Thread 9ab5d50a、R-001、B-001。ルート: 委任。
- `ApplicationQuitIntentPort` の具体実装の位置: domain の `ApplicationQuitIntentPort` の具体実装（Tauri の exit / restart と tray への変換）を gateway の責務の位置に置き、controller には入口の受付と配線だけを残す。debug desktop 経路での終了・再起動の挙動は変えない。開始状態では `src-tauri/src/adaptor/controller/application_lifecycle.rs:37-54` の `TauriApplicationQuitIntentPort` が domain の trait を実装しており、同じ port のもう一つの具体実装 `DaemonProcessActionPort` は `adaptor/gateway/application_lifecycle.rs:8-19` にあって、同種の変換が 2 層に分散している。根拠: Thread f60fcd49、規約 `docs/architecture/GATEWAY.md`、`docs/architecture/CONTROLLER.md`。ルート: 委任。
- 本番の terminal 停止を通す終了経路の検証: 終了経路の terminal の停止を async 文脈で直接ブロックさせる退行で検証が失敗するようにする。開始状態では `src-tauri/src/adaptor/controller/daemon_test.rs:58-167` が `Daemon::wait` の組み立て・完了通知・終了コード 23・15 秒の打ち切りを子プロセスで検証するが、注入するのは `usecase/application_lifecycle/test_helpers.rs:8-57` の `FakeShutdown`（`tokio::time::sleep` と `std::future::pending` による協調的 future）であり、`adaptor/gateway/application_lifecycle.rs:47-53` の `terminal.shutdown()` を `spawn_blocking` に載せる本番経路を通らない。`adaptor/gateway/application_lifecycle_test.rs` の 2 件は `shutdown_telemetry` のみを対象とする。根拠: Thread eb399c04、R-002、R-003、B-002、B-003。ルート: 委任。

## 固定するルート

- 今周に新しく固定する実装上の指定なし。
- design-01 のルート1〜ルート5 は今周も維持する（解除しない）。とくにルート3 が明示する DROP 対象（`operation_bindings`、`caller_attempts`、`operation_records`、`obligations`、`pending_obligations`、`recovery_action_attempts`、`shutdown_plans`、`shutdown_targets`、`shutdown_recovery_snapshots` と `store_metadata` の `current_shutdown_id`、`shutdown_pointer_revision`）と、ルート5 が定める「終了処理でターミナルを止める前に provider exit observer を止める」順序は今周も変更しない。
- design-02 のルート6・ルート7 は今周も維持する（解除しない）。ルート6 の 2 点（`log::logger().flush()` を終了処理の直列段階に戻さない、`drop(telemetry)` による OTLP shutdown は 15 秒の上限の内側に残す）と、ルート7 の variant 名（`ProviderSessionOwnership` / `ProviderLifecycle` / `ProviderHookHealth`）を変更しない。

## 変えないもの

- `requirements.md` と `behavior.md`。範囲: 今周の全体。理由: 人間が今周で変更しないと決めたため。
- 打ち切り経路でのログの永続化を要求として追加しないこと、および終了処理のための新しいログ保存機構を追加しないこと。範囲: 終了処理のログ。理由: 上限の外に出せば終了しない経路を新たに作るため（Thread 3125c385 についての人間の決定を維持する）。
- 永続形式の文字列（`envelope.rs` の `provider-lifecycle` 等）。範囲: `LocalDomainEvent` の variant 名。理由: store の互換に影響させないため。

## 未確定・リスク

- 自動判断（`requirements.md` の Assumptions、Design 01 で記録）: R-001 の「プロセスの終了」に、プロセスを終了するときの local API の停止を含めた。
- 自動判断（`requirements.md` の Assumptions、Design 01 で記録）: R-006 の outcome unknown の表示を終了のバナーの表示に限り、UI の daemon 監督の `Daemon shutdown outcome is unknown; switching is blocked.` は変える部分に含めていない。
- 未決のまま残した要求（Assumptions の「自動判断: 未決」）は無い。今周に `[DEFERRED]` で人間へ渡した件も無い。
