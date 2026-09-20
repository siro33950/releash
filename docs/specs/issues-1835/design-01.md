# Design 01

## 開始状態

- 差分の基準: `main` から派生した `feat/issues/1835`。派生点は `2774f470`（release: v0.4.14）。未コミットの変更は `docs/specs/issues-1835/` の文書だけで、コードは派生点のまま。
- 直前の Design: なし（初回）。変更前の挙動は `requirements.md` の Current Behavior を参照する。
- Thread: この周の開始時点で open Thread は無い。

## 変える部分

- 終了処理の内容の置き換え: daemon の終了処理を、workflow が起動した command の停止、ターミナルの停止と状態の保存、プロセスの終了（local API の停止を含む）だけにし、ShutdownCoordinator による対象一覧の確定と対象ごとの処理（`src-tauri/src/usecase/shutdown_coordinator.rs`、`src-tauri/src/adaptor/controller/application_lifecycle.rs` の `RuntimeShutdownExecutor`）を経由しない。根拠: R-001「終了処理が行うのは、workflow が起動した command の停止、ターミナルの状態の保存、プロセスの終了だけである」、B-001。ルート: ターミナルを止める前に provider exit observer を止める（ルート5）。workflow の状態は終了処理で記録しない（ルート4）。command の停止とターミナルの状態の保存の組み立ては委任。
- 段階の失敗で止まらない終了: 終了処理のどの段階が失敗してもログに出して次の段階へ進み、プロセスを終了する（現行は、対象の失敗で `ReconciliationRequired` を保存し後続の段階を行わず、ターミナルの停止が失敗すると local API の停止を行わない。`application_lifecycle.rs:133-142`）。根拠: R-002「失敗をログに出力して次の段階へ進み、プロセスを終了する」、B-002。ルート: 委任。
- 15 秒の上限: Quit、更新を伴わない再起動、更新の適用のいずれによる終了処理にも全体で 15 秒の上限を設け、超えたら残りの処理を打ち切ってプロセスを終了する（現行はターミナルの停止が出力の終わりを上限なく待ち、再起動・更新には UI 側の期限が無い）。根拠: R-003「終了処理の全体に 15 秒の時間の上限を設け、上限を超えた場合は残りの処理を打ち切ってプロセスを終了する」、B-003。ルート: 委任。
- 終了手続きの記録と対象ごとの進捗管理の削除: shutdown plan、shutdown target、recovery snapshot、現在の終了手続きの指し先、対象ごとの進捗と再試行の記録、command の停止に伴う workflow shutdown の obligation 記録（`src-tauri/src/adaptor/gateway/workflow/runtime_command_gateway.rs:446-550`、`src-tauri/src/domain/local_event/workflow_shutdown.rs`）を、保存・読み出しの処理ごと削除する。根拠: R-004「終了処理は、終了手続きの記録、止める対象ごとの進捗…を保存しない」、B-001。ルート: ルート1。
- quit の呼び出し記録の削除: 終了要求の caller attempt、operation binding、operation record（`src-tauri/src/usecase/application_lifecycle/operation/`）を、保存・読み出しの処理ごと削除する。根拠: R-004「終了要求の呼び出し記録を保存しない」、B-001。ルート: ルート1。
- 書き込みの拒否の削除: store の書き込みの受付判定から、終了手続きの記録を理由とする拒否（`src-tauri/src/adaptor/gateway/local_event_store/commit.rs:663-777`、`src-tauri/src/domain/local_event/commit_admission.rs`）を削除する。根拠: R-005「前回の終了処理がどの段階で途切れていても、…書き込み操作は、前回の終了を理由に拒否されない」、B-004、B-005。ルート: ルート1、ルート2。
- schema v8: 使われなくなるテーブルと `store_metadata` の列を schema v8 の migration で DROP する。根拠: R-004、R-005（人間の指定したルート3）。ルート: ルート3。
- 終了のバナーの削除: `Retry same effect`、`Retry quit`、outcome unknown を表示する終了のバナー（`src/components/layout/ApplicationShutdownBanner.tsx`、`src/hooks/useApplicationShutdownSupervision.ts`、localStorage の `releash:application-quit-attempt:v1`）と、バナーだけが使う終了関連の読み取り・再試行・確認の API（Connect の `GetApplicationQuitOperation`、`GetApplicationShutdown`、`GetShutdownPlan`、`ResolveShutdownTargetAction`、`ListPendingApplicationAttempts`、`AcknowledgeApplicationAttempt`、`CompactApplicationShutdownDetails`）を削除する。根拠: R-006「終了処理に関する画面（`Retry same effect`、`Retry quit`、outcome unknown の表示）を持たない」、B-006。ルート: ルート1。API の削除の方法は委任。
- 前回の終了の解決を求める分岐の削除: 終了要求に対する前回の終了の解決が必要という応答（`shutdown_coordinator.rs:3024-3026`）と、UI 側の `Previous shutdown requires a decision before switching.` への変換と処理（`src-tauri/src/adaptor/gateway/daemon_supervision.rs:336-344`、`src-tauri/src/domain/daemon_supervision.rs` の `ShutdownResponse::DecisionRequired`）を削除する。根拠: R-006「終了・再起動・更新の要求は、前回の終了の解決を求めて止まらない」、B-006。ルート: ルート1。

## 固定するルート

- ルート1: 次の仕組みを削除する（無効化や迂回はしない）。対象: 終了手続きの DB 記録と対象ごとの進捗管理、「記録が未完了なら書き込みを拒否する」判定、Retry same effect / Retry quit / outcome unknown の画面、Previous shutdown requires a decision の分岐、quit の呼び出し記録。粒度: 仕組みの単位での削除。手順と順序は委任。理由: ISSUE #1835「この仕組みは今は不要」「削除するもの」。関係: R-004、R-005、R-006（B-001、B-004、B-005、B-006）。
- ルート2: 詰まった store を使えるようにするための起動時の復旧処理は追加しない。拒否の判定を削除することで使えるようにする。粒度: 復旧処理を足さない、という指定だけ。理由: ISSUE #1835「起動時に追加するものは無い」。関係: R-005（B-004、B-005）。
- ルート3: 使われなくなるテーブル（`operation_bindings`、`caller_attempts`、`operation_records`、`obligations`、`pending_obligations`、`recovery_action_attempts`、`shutdown_plans`、`shutdown_targets`、`shutdown_recovery_snapshots`）と、`store_metadata` の列（`current_shutdown_id`、`shutdown_pointer_revision`）を、schema v8 の migration で DROP する。粒度: 対象のテーブルと列を指定。migration の書き方（`store_metadata` の作り直しを含む）は委任。理由: 使われない schema をデッドコードとして残さない。これは復旧処理ではなく schema の更新で、ルート2 と両立する。関係: R-004、R-005。
- ルート4: 終了時に実行中だった workflow の状態は、既存の起動時の処理（Session / Command Node にプロセスの喪失を記録する処理。`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:835-866`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:645-660`）に任せる。終了処理では記録しない。粒度: 既存の処理を使い、新しく作らない。理由: ISSUE #1835「既存の処理が拾う」。関係: R-007（B-007）。
- ルート5: 終了処理でターミナルを止める前に、provider exit observer を止める（現行どおり）。粒度: 順序の指定だけ。理由: 終了のためにターミナルを止めたことを AgentSession のプロセス終了として記録しないため。関係: R-001（B-001）。

## 変えないもの

- 終了処理の中でターミナルを止める前に provider exit observer を止める、現行の順序。範囲: 終了処理によるターミナルの停止。理由: 再起動後、provider session id を持つ Session を開くと自動で resume される今の挙動を保つため。

## 未確定・リスク

- 自動判断（`requirements.md` の Assumptions）: R-001 の「プロセスの終了」に、プロセスを終了するときの local API の停止を含めた。
- 自動判断（`requirements.md` の Assumptions）: R-006 の outcome unknown の表示を終了のバナーの表示に限り、UI の daemon 監督の `Daemon shutdown outcome is unknown; switching is blocked.` は変える部分に含めていない。
- 終了完了の出力: UI の daemon 監督は、再起動・更新の要求の後に daemon が `releash-shutdown-complete` を出力せずに終了すると、切替を止めて `Daemon shutdown outcome is unknown; switching is blocked.` を表示する（`src-tauri/src/domain/daemon_supervision.rs:280-283`、`:378-388`）。現行の daemon はこの出力を local API の停止に成功した後にだけ行う（`src-tauri/src/adaptor/controller/daemon.rs:16-24`）。15 秒の上限による打ち切りや段階の失敗の経路でこの出力が出ないと、再起動・更新の場合に R-002・R-003（B-001〜B-003）を満たせない。
