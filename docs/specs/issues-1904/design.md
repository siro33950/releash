# Design

パスは、`proto/`・`src/`（プロジェクトルートの画面）・`tests/`（プロジェクトルート）で始まるもの以外は `src-tauri/` 起点。

## 変える部分

- サーバの起動の処理: `releash-sdk` に、同じディレクトリの `releashd` を detached で起動する処理を作る（`setsid`、stdin・stdout を閉じる、stderr は data dir の `logs/` のファイルへ起動ごとに上書きで redirect、起動元の cwd を渡す、data dir を明示して渡す）。起動に失敗したら（発見ファイルが出る前にプロセスが終わったら）終了の状態とそのファイルの末尾を返す。待ちの上限は新しい定数を作らず、`proto/client.proto` の `ClientService` の service option `min_connect_timeout_ms` を `releash-sdk` の descriptor から読んで使う。上限を超えたら、プロセスを止めずに「起動を確認できなかった」としてその時点の stderr のファイルの末尾を返す。根拠: R-002「同じディレクトリの `releashd` を起動して…画面から独立したプロセスとして動く」、R-003、B-002、B-003。ルート: 処理は `releash-sdk` の 1 つにし、#1905 の CLI も同じものを使う。ファイル名は委任。`setsid` に `libc` を使う場合は `deny.toml` の allow list を確かめる。
- 画面の起動の流れ: シェルが `releash-sdk` で発見（`discovery::read_optional`・`verify_process`）→ `GetServerInfo` と `verify_server` → `Compatibility::assess` → 居なければ上の起動処理 → 接続先の受け渡し、の順で行う。互換が無い・起動できないときは失敗の窓を出す。根拠: R-001〜R-004、B-001〜B-006。ルート: 隣の `releashd` の探し方は今のまま（`releash-desktop/src/desktop.rs:70` の `current_exe().with_file_name("releashd")`）。失敗の窓の作り方は委任。
- 古いサーバの入れ替え: 失敗の窓（サーバが古いとき）に、`StopDaemon` を呼んでプロセスの終了と発見ファイルの消失を待ち、上の起動処理で起動し直す操作を置く。根拠: R-004、B-005。ルート: 停止はトレイの「サーバを停止」と同じ 1 つの処理を使う（入口が 2 つ、処理は 1 つ）。待ち時間の上限は #1908 の `shutdown_timeout_ms`。
- 監督の削除: `releash-desktop/src/{domain,usecase,adaptor/gateway}/daemon_supervision.rs` とそのテスト、`adaptor/presenter/daemon_status.rs`、`adaptor/controller/desktop_lifecycle.rs` の監督の部分（`observe`・`observe_with`・`DesktopHost`・`request_quit`）、`adaptor/controller/application_lifecycle.rs`（`ApplicationQuitIngress`）、`adaptor/controller/command/mod.rs:17-56` の監督による受付の判定、Tauri コマンド `get_daemon_status`・`subscribe_daemon_status`・`stop_daemon_status_subscription`・`retry_daemon`・`validate_daemon_connection`・`restart_desktop` を消す。根拠: R-005、R-007「画面は自分からサーバを起動し直さず、サーバを強制終了しない」、B-009。ルート: `get_client_endpoint`（接続先の受け渡し）と `quit_desktop`（画面だけを閉じる）は残す。`get_client_endpoint` の返り値から `launch_id` を消す。消すコマンドを使うテスト（`releash-desktop/tests/desktop_daemon.rs`、`tests/daemon_termination.rs`、`tests/desktop_status_channel.rs`、`tests/internal/daemon_supervision.rs` ほか）は消す機能のテストとして消す。
- Rust 側の死活監視の削除: `desktop_client.rs` の `DaemonLiveness` による判定（`releash-desktop/src/adaptor/gateway/desktop_client.rs:193-219` と `domain/daemon_supervision.rs:5-20`）を消し、生存の判定は TS の `src/lib/client.ts` だけにする。根拠: R-007、B-009。ルート: 委任。
- 画面の「動いていない」表示と起動: `src/components/DaemonBoundary.tsx` の監督の状態の表示（`subscribe_daemon_status`・Retry・Phase による overlay）を消し、TS の接続の状態（`src/lib/client.ts:34-38`）から「動いていない」と、サーバを起動する操作を出す。起動の操作は Tauri コマンドでシェルの起動の処理（上の `releash-sdk` の処理）を呼ぶ。根拠: R-007、B-009、B-010。ルート: 自動では起動し直さない。起動の処理は起動時と共有する。表示の形と文言は委任。
- 接続の確立: `src/lib/client.ts:68-125` の `validate_daemon_connection` の呼び出しを消す。根拠: R-008（`launch_id` の削除）、B-012。ルート: 委任。
- Quit: トレイの Quit、ネイティブ終了、`RunEvent::ExitRequested`、`quit_desktop` を、サーバを止めずに画面のプロセスを終える処理にする。根拠: R-005、B-007。ルート: 委任。
- トレイの「サーバを停止」: トレイのメニュー（`releash-desktop/src/infrastructure/platform/tray.rs`）に足し、動いている agent の Session も止まる旨の確認の後に `StopDaemon` を呼び、プロセスの終了と発見ファイルの消失を待つ。根拠: R-006、B-008。ルート: 画面の操作はトレイの 1 か所だけ。確認の文言は委任だが、動いている agent の Session も止まることを含める。止めた後の画面は「動いていない」の表示になる（B-009 と同じ）。
- `StopDaemon`: `proto/client.proto` に `rpc StopDaemon(StopDaemonRequest) returns (StopDaemonResponse)`（scope は operator、request は field 無し）を足し、`CommandRequest`／`CommandResult` の oneof に載せる。`RequestApplicationQuit`・`RequestApplicationQuitRequest`・`ApplicationQuitRequestDtoV1`・`ApplicationQuitIntentDtoV1`・`ApplicationQuitOutcomeDtoV1` を消し、oneof 113 の番号と名前を reserved にする。サーバは今の `request_application_quit_shared` の Exit の経路（`Daemon::stop` → process port。`src/adaptor/controller/client/application_lifecycle.rs:10-29`）で終了コード 0 にし、Restart の経路は消す。受付の表（`src/adaptor/controller/api/client_admission.rs:8`）は `"StopDaemon" => DaemonRequest::Stop` にする。根拠: R-008、B-011、B-012。ルート: AIP-180 に従い別名にする。domain の `StopRequest::Exit { code }`（`src/domain/daemon/mod.rs:25,81`）は、呼び出し元が 0 しか渡さなくなるなら `code` を持たない形にしてよい（読んで決める）。TS の生成物は `pnpm generate:protocol` で作り直す。
- `ServerInfo.launch_id`: `proto/client.proto:2598` の field 2 を消して番号と名前を reserved にし、`src/adaptor/controller/api/client_service.rs:6-7` と `src/adaptor/presenter/daemon.rs` の `launch_id` を消す。根拠: R-008、R-011、B-012。ルート: 委任。
- 更新: `releash-desktop/src/usecase/desktop_update.rs` の `apply` を、ダウンロード → インストール → 画面の起動し直し（`infrastructure/platform/desktop_restart::restart`）にし、監督への依存を消す。根拠: R-009、B-013。ルート: `desktop_restart::restart` は残す。`DesktopUpdateInstaller`（`domain/daemon_supervision.rs:38-43`）は更新の側へ移す（置き場所は委任）。
- desktop 設定と ログイン項目: シェルの `desktop_client.rs` の接続と `desktop-settings` の購読から、close_to_tray・telemetry・`start_minimized`・`auto_launch` を受け取り、ログイン項目の希望の保存（`UpdateLoginItemPreference`）も同じ接続で行う。`desktop.rs:66-68,77` の `releash.toml` の直接読みを消し、`start_minimized` とログイン項目の復元は購読の最初の状態から取る。接続できるまでは既定値（`--hidden` の起動では窓を出さない、ログイン項目は触らない）。根拠: R-010、B-014、B-015。ルート: 監督を通さず、`desktop_client.rs` の接続と購読から直接取る。`login_item.rs:55-83` の `DaemonLoginPreference` は `DaemonProcessGateway` に依存しない形にする。接続に失敗したら失敗の窓を出す。
- サーバ側の子プロセス向けの仕組みの削除: `src/bin/backend.rs:14-16` の `--internal-daemon`、`src/infrastructure/process/parent_lifetime.rs`（`watch_parent_pipe`・`CHILD_SPAWNS`・`spawn_guard`・`terminate_descendants`）と `src/lib.rs:35` の呼び出し、5 か所の `spawn_guard()` の呼び出し（`native_pty.rs:245`、`command_runner.rs:138`、`search_path.rs:72`、`output.rs:39-41`、`background_worker.rs:48`）、`src/desktop_api.rs:48` の再公開、`src/adaptor/controller/daemon.rs:94` の完了マーカーを消す。根拠: R-011、B-016。ルート: `parent_lifetime.rs` が空になればファイルごと消す。`--internal-background-worker` と `--data-dir` は残す。
- テスト: `--internal-daemon`・`RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE`・完了マーカーを使うテスト（`tests/daemon_smoke.rs:184-202,260-298,353-381,408,1228`、`tests/internal/adaptor_controller_daemon.rs:84`、`releash/tests/support/diagnostics.rs:42-43`、`releash-desktop/tests/support/mod.rs`、`tests/helpers/client-recovery.mjs`・`tests/helpers/desktop-daemon.mjs`、`src/test/connect.ts`）を、引数なし・`--data-dir` の起動と発見ファイルの消失での停止の観測に追従させる。消す機能だけを確かめるテストは消す。根拠: R-008、R-011。ルート: 委任。

## 固定するルート

- サーバの detached の起動は `releash-sdk` の 1 つの処理にし、画面の起動時・画面の「起動」の操作・失敗の窓の入れ替え・#1905 の CLI が同じものを使う。stderr は data dir の `logs/` のファイルへ起動ごとに上書きで redirect し、stdin・stdout は閉じる。
- 隣の `releashd` は `current_exe().with_file_name("releashd")` で探す。
- 画面の「サーバを停止」はトレイの 1 か所。失敗の窓の「停止して起動し直す」は同じ停止の処理の別の入口。
- サーバ側の停止で terminal の出力 drain を待つ処理（`adaptor/gateway/terminal_surface/runtime_gateway_impl.rs` の `wait_for_output_drain`）は 2 秒を上限にする。drain は PTY の EOF で完了するが、shell の background job が PTY を掴んだままだと Linux では EOF が来ず、停止が `shutdown_timeout_ms` の 15 秒まで塞がる（main では親 EOF 経路の子孫終了がこれを隠していた。CI の `tests/daemon_smoke.rs` の「stdin を閉じても動き続ける」で露見）。上限を過ぎたら警告を記録して停止を続ける。
- 停止の完了を待つ処理（`StopDaemon` を呼び、プロセスの終了と発見ファイルの消失を待つ。上限は #1908 の `shutdown_timeout_ms`）は `releash-sdk` の 1 つの処理にし、トレイの「サーバを停止」・失敗の窓の「停止して起動し直す」・#1905 の CLI `server stop` が同じものを使う。
- 生存の判定は TS の `src/lib/client.ts` だけ。
- desktop 設定とログイン項目は、シェルの `desktop_client.rs` の接続と購読から直接取る。
- `get_client_endpoint` と `quit_desktop` は残す。`desktop_restart::restart` は残す。
- 接続の操作（初期化・接続先の受け渡し・起動・入れ替え）と設定の変化の適用の直列化は、common の 1 つの入口（`Serial` の中で処理を呼び、結果を接続後の処理に渡す）で掛け、各入口はそれを呼ぶ。Tauri の async コマンドは `CommandRouter::handle` の先で spawn されるため、ルータの手前では処理の終わりまで直列化を保てない。
- `StopDaemonRequest` は field を持たず、終了コードは 0。`RequestApplicationQuit` 系と `ServerInfo.launch_id` は reserved。

## 変えないもの

なし

## 未確定・リスク

- 更新の後は新しい画面と古い `releashd` が動く。protocol が同じ（package が同じ v1）なら互換と判定されて接続するので、サーバが新しい版になるのは利用者が「サーバを停止」して起動し直したときだけになる。
