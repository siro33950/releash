# Design 02

## 開始状態

- 差分の基準: `main` から派生した `feat/issues/1835`。派生点は `2774f470`（release: v0.4.14）。
- 直前の Design: `docs/specs/issues-1835/design-01.md`。同 Design の周の実装は未コミットの作業ツリーに入っており、このコードを今周の開始状態とする。
- Requirements・Behavior: 今周は変更しない。`requirements.md` の R-001〜R-007、`behavior.md` の B-001〜B-007 と対応表は据え置く。
- Thread: この周までに解消・見送りとなった Thread は無い。`[FIX_POLICY]` が付いた open Thread が 10 件あり、今周で変える部分はこの 10 件である（3125c385、40cf5057、fb7b03d4、9827a897、eb399c04、1d9f51b2、2786eb17、4d52752c、876360bf、4a5765cc）。`[REJECTED]`・`[DEFERRED]` と判定された Thread は無い。

## 変える部分

- 終了処理の直列段階からの log flush の除外: `log::logger().flush()` を終了処理の直列段階から外し、ログの書き込みは既存の専用 writer スレッド（`src-tauri/src/infrastructure/local_log.rs`）による並行書き込みに任せる。`drop(telemetry)` による OTLP の tracer / meter / logger の shutdown（`src-tauri/src/infrastructure/telemetry/mod.rs:20-26`）は終了処理の段階として 15 秒の上限の内側に残す。開始状態では `daemon.rs:44` の `flush_shutdown` が `finish_shutdown` の 15 秒 timeout の内側で `drop(telemetry)` と `log::logger().flush()` を続けて実行する（`daemon.rs:52-61`）。根拠: Thread 3125c385。ルート: ルート6。呼び出しの配置と `Daemon::wait` / `usecase::application_lifecycle::shutdown` の組み立て方は委任。
- `LocalDomainEvent` の variant 名の差し戻し: `src-tauri/src/domain/local_event/events.rs:14-18` の `SessionOwnership` / `Lifecycle` / `HookHealth` を `ProviderSessionOwnership` / `ProviderLifecycle` / `ProviderHookHealth` へ戻し、Rust の実装・テストの使用箇所を追随させる。根拠: Thread 40cf5057。ルート: ルート7。進め方と使用箇所の追随の仕方は委任。
- command 停止の OS エラーログの検証: `src-tauri/src/infrastructure/process/child_process.rs:33-35` / `:41-43` / `:52-58` / `:64-66` / `:72-78` / `:83-85` の wait / signal / start_kill / reap の失敗分岐から診断ログが出ることを、その分岐を通す検証で確認できるようにする。開始状態の同ファイルのテストは `signal_process_group` の pgid ガード 2 件だけで、いずれのログ分岐も通らない。根拠: Thread fb7b03d4、R-002、B-002。ルート: 委任。
- 起動成功と停止の競合の検証: `src-tauri/src/adaptor/gateway/workflow/workflow_host/shutdown_test.rs` の競合テストが、spawn が成功する入力で `spawn_command_execution` と `shutdown_all_active_commands` の競合を通り、登録された command が停止対象から漏れないことを検出できるようにする。開始状態の入力は `shutdown_test.rs:22` の `"invalid\0command"` で spawn が必ず失敗し、`workflow_host.rs:1607` 以降の登録経路を通らない。根拠: Thread 9827a897、R-001、B-001。ルート: 委任。
- 終了経路そのものの検証: `Daemon::wait` の実際の組み立て（`daemon.rs:22-48`）で、段階が失敗または停止しても 15 秒で打ち切り、`releash-shutdown-complete` を出力し、要求された終了コードでプロセスが終了することを検証できるようにする。開始状態の `daemon_test.rs` は `finish_shutdown` と `flush_shutdown` を個別に検証するだけで、`Daemon::wait` の組み立て・完了通知・終了コードを通らない。根拠: Thread eb399c04、R-002、R-003、B-002、B-003（design-01 の未確定・リスクの「終了完了の出力」に対応する）。ルート: 委任。
- `Daemon::wait` の契約と呼び出し側の整合: 正常終了時に呼び出し元へ戻らないという契約を `Daemon::wait` のシグネチャに表し、`src-tauri/src/lib.rs` 側に到達不能な成功分岐を残さない。開始状態では `daemon.rs:17` が `Result<i32, String>` を返しながら成功時は `:48` の `std::process::exit(code)` に進み、`lib.rs:49-56` の flush と `Ok(code) => code` が到達しない。exit channel の受信失敗（`daemon.rs:18`）のエラー経路の扱いは維持する。根拠: Thread 1d9f51b2、R-001、B-001。ルート: 委任。
- schema v8 の DDL と移行機構の配置: 今周新設した v8 の `CREATE` / `ALTER` / `DROP` と移行トランザクションの機構（`src-tauri/src/adaptor/gateway/local_event_store/schema.rs:181-219`、`:229-260`）を infrastructure の責務として置き、gateway 側には DML と保存表現 ↔ domain record の変換を残す。対象は今周新設した v8 の処理に限り、既存 schema 全体の移設は行わない。根拠: Thread 2786eb17、規約 `docs/architecture/INFRASTRUCTURE.md:16`、`:21`。ルート: 委任（ルート3 の DROP 対象は変更しない）。
- 終了要求の入口と終了手順の所有: 終了要求が controller 定義の port（`src-tauri/src/adaptor/controller/application_lifecycle.rs:24-26` の `ApplicationQuitIntentPort`）へ直接受理される経路（`client/application_lifecycle.rs:29-39`）を、内側の application lifecycle 境界を通すようにし、終了手順の全体（段階の順序、段階失敗後の継続、全体の時間の上限）の所有を `docs/architecture/CONTROLLER.md:7`、`:13` と `USECASE.md:19` の責務配分に一致させる。開始状態では `daemon.rs:22-46`、`:63-71` が終了 usecase の呼び出しと 15 秒の打ち切りを controller 側で組み立て、`usecase/application_lifecycle/mod.rs:3-19` が持つのは渡された 4 段階の順序と失敗ログだけである。根拠: Thread 4d52752c。ルート: 委任（design-01 のルート1〜ルート5 は解除しない）。
- 終了開始後の command 起動受理の所有者: 終了開始後に command の起動を受け付けないという状態と受理規則の所有者を domain として実行経路に現し、gateway 内の `Arc<Mutex<bool>>`（`workflow_host.rs:118`、`:450`、`:1603-1606`、`:2002-2003`）が受理判定の所有者でなくなるようにする。R-001 / B-001 が求める起動と停止の排他自体は維持する。根拠: Thread 876360bf、規約 `docs/architecture/GATEWAY.md:11`、`:12`。ルート: 委任。
- 用途を失った `store_metadata` の 3 値の削除: `cursor_hmac_key` / `operation_binding_hmac_key` / `process_instance_id` の生成、保存、v8 へのコピー、open 時の更新、形式検証（`schema.rs:188-190`、`:200-206`、`:221-253`、`:751-786`、`store.rs:498`、`:542-545`、`:589-592`、`:636-639`）を store の作成・起動経路から削除する。各値の消費先は今周で削除済みである（`operation_binding_hmac_key` は基準 `2774f470` の `usecase/application_lifecycle/operation/ports.rs:13` の `OperationBindingAuthority`、`cursor_hmac_key` は `cursor.rs`、`process_instance_id` は基準の `controller/application_lifecycle.rs:267`）。R-005 / B-005 の「この変更より前の版の store を、利用者の操作や store の直接操作なしにそのまま使える」は維持する。根拠: Thread 4a5765cc、design-01 のルート1「無効化や迂回はしない」「仕組みの単位での削除」、ルート3 の理由「使われない schema をデッドコードとして残さない」、R-004。ルート: 委任（ルート3 が明示する DROP 対象は変更しない）。

## 固定するルート

- ルート6（今周に固定）: 終了処理の段階の構成について 2 点を指定する。(1) `log::logger().flush()` を終了処理の直列段階から外す。(2) `drop(telemetry)` による OTLP shutdown は終了処理の段階として 15 秒の上限の内側に残す。粒度: この 2 点だけ。実装の書き方（呼び出しの配置、`Daemon::wait` と `usecase::application_lifecycle::shutdown` の組み立て方）は委任。終了処理のための新しいログ保存機構は追加しない。関係: Thread 3125c385、R-002、R-003（B-002、B-003）。
- ルート7（今周に固定）: `LocalDomainEvent` の variant 名を `ProviderSessionOwnership` / `ProviderLifecycle` / `ProviderHookHealth` へ戻す。粒度: variant 名の差し戻しと Rust の使用箇所の追随のみ。進め方は委任。関係: Thread 40cf5057。
- design-01 のルート1〜ルート5 は今周も維持する（解除しない）。とくにルート3 が明示する DROP 対象（`operation_bindings`、`caller_attempts`、`operation_records`、`obligations`、`pending_obligations`、`recovery_action_attempts`、`shutdown_plans`、`shutdown_targets`、`shutdown_recovery_snapshots` と `store_metadata` の `current_shutdown_id`、`shutdown_pointer_revision`）は今周も変更しない。

## 変えないもの

- `requirements.md` と `behavior.md`。範囲: 今周の全体。理由: 打ち切り経路でのログの永続化を要求として追加すると、上限の外に出せば終了しない経路を新たに作るため（Thread 3125c385 についての人間の決定）。
- 永続形式の文字列（`envelope.rs` の `provider-lifecycle` 等）。範囲: ルート7 の variant 名の差し戻し。理由: store の互換に影響させないため。
- 終了処理の中でターミナルを止める前に provider exit observer を止める、現行の順序（design-01 の「変えないもの」を維持）。範囲: 終了処理によるターミナルの停止。理由: 再起動後、provider session id を持つ Session を開くと自動で resume される今の挙動を保つため。

## 未確定・リスク

- 自動判断（`requirements.md` の Assumptions、Design 01 で記録）: R-001 の「プロセスの終了」に、プロセスを終了するときの local API の停止を含めた。
- 自動判断（`requirements.md` の Assumptions、Design 01 で記録）: R-006 の outcome unknown の表示を終了のバナーの表示に限り、UI の daemon 監督の `Daemon shutdown outcome is unknown; switching is blocked.` は変える部分に含めていない。
- 未決のまま残した要求（Assumptions の「自動判断: 未決」）は無い。`[DEFERRED]` で人間へ渡した件も無い。
