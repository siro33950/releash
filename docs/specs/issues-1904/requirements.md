# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1904 （[07] 画面をサーバの利用者にし、Quit は画面だけを閉じる）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、#1903（`docs/specs/issues-1903/`）、#1908（`docs/specs/issues-1908/`）、#1905（[08] CLI の status / server コマンド）
- 本文の「今の作り」の file:line は main `4533af2f` 時点のもの。この文書は main `61336b06`（#1903 の後）で読み直した事実に基づく。
- 本文の「`ClientConnectionQueryService` の実装が 2 つ」は解消済み。実装は `src-tauri/src/adaptor/gateway/local_api.rs:9` の 1 つだけで、画面はそれを直接呼んでいる（`src-tauri/releash-desktop/src/adaptor/gateway/daemon_supervision.rs:79`）。
- 本文の方針「`spawn_guard` は残す」とは違い、`spawn_guard`・`CHILD_SPAWNS`・`terminate_descendants` も消す。`spawn_guard()` は `CHILD_SPAWNS` の read lock を返すだけで（`src-tauri/src/infrastructure/process/parent_lifetime.rs:4-8`）、write lock を取るのは消す `watch_parent_pipe` の 1 か所だけ（同 `:28`）のため、親 pipe を消すと何も待たない lock になる。`terminate_descendants`（同 `:34`）の使い手も、消す `watch_parent_pipe` と画面の監督（`src-tauri/releash-desktop/src/adaptor/gateway/daemon_supervision.rs:273`）だけになる。
- 本文の未決「画面を開いている間にサーバが居なくなったとき」は、「動いていない」と表示し、利用者の操作で起動できるようにすると決めた。自動では起動し直さない。
- 互換は proto の package の版（`src-tauri/releash-sdk/src/descriptor.rs:27-36`。今は v1）で判定する。画面の更新は動いている `releashd` を止めないので、更新の後は新しい画面と古い `releashd` が共存する。
- 対応プラットフォームは macOS（AGENTS.md「リリース」）。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。サーバを通らずに記録を読み書きする経路は無い。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-7a から受けた。

# Outcome

- 対象者: Releash の画面（Tauri アプリ）を使う人、CLI・hook・agent を使う人、Releash の開発者。
- 現在の問題: サーバは画面の子プロセスで、画面を Quit するとサーバと動いている agent も止まる。画面がサーバを監督し（起動し直し、強制終了、死活監視）、その仕組みが画面とサーバの両方にある。画面を閉じたあとにサーバを残す手段が無い。
- 変更後の状態: 画面はサーバの利用者になる。起動時に、動いているサーバを見つけて互換を判定し、居なければ起動して接続する。Quit は画面だけを閉じ、サーバと agent は動き続ける。サーバを止めるのは画面の「サーバを停止」だけで、agent も止まる旨を確認してから止める。サーバ側から、画面の子プロセスとして動くための仕組みが無くなる。

# Current Behavior

main `61336b06` で読んで確かめた挙動。パスは `src-tauri/` 起点。

- 画面の起動: シェルは自分の隣の `releashd` を `--internal-daemon <data_dir>`、env `RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE=1`、stdin・stdout・stderr を pipe で起動する（`releash-desktop/src/desktop.rs:70`、`releash-desktop/src/adaptor/gateway/daemon_supervision.rs:163-225`）。既に動いているサーバには接続しない（発見ファイルの pid が自分の子と違えば失敗。同 `:347-360`）。
- 監督: `releash-desktop/src/{domain,usecase,adaptor/gateway}/daemon_supervision.rs` が 7 状態の Phase（`domain/daemon_supervision.rs:46-54`）で起動・再試行（1/2/4 秒で 3 回。`:4`）・強制終了（`adaptor/gateway/daemon_supervision.rs:262-280`）・終了の観測を行う。状態は `subscribe_daemon_status` で画面に配信され、`src/components/DaemonBoundary.tsx` が「Starting Releash…」等と Retry / Quit を出す。
- 死活監視: シェルの Rust（`releash-desktop/src/adaptor/gateway/desktop_client.rs:193-219,317-391`）と TS（`src/lib/client.ts:242,354-439`）の両方が状態の stream の沈黙で判定する。
- 接続の確立: TS は Tauri コマンド `get_client_endpoint`（`releash-desktop/src/adaptor/controller/command/client.rs:24-36`）→ `GetServerInfo` → `validate_daemon_connection`（launch_id と release の完全一致。`releash-desktop/src/domain/daemon_supervision.rs:407-428`）の順に呼ぶ（`src/lib/client.ts:68-125`）。
- Quit: トレイ（`desktop.rs:101-104`）、ネイティブ終了（`:108-111`）、`ApplicationQuitIngress`（`:112-121`）、`quit_desktop`（`adaptor/controller/command/desktop_lifecycle.rs:70-73`）、`RunEvent::ExitRequested`（`adaptor/controller/desktop_lifecycle.rs:118-132`）がすべて `supervisor.stop(StopIntent::Quit)` に集まり、`RequestApplicationQuit` でサーバを止めてから画面が終わる。
- 更新: `install_desktop_update` はダウンロード → サーバの停止を待つ → インストール → 画面を起動し直す（`releash-desktop/src/usecase/desktop_update.rs:48-70`）。
- desktop 設定: シェルは監督が持つ接続で `desktop-settings` を購読し（`desktop_client.rs:254-269`）、`DaemonConnection.settings` 経由で close_to_tray・telemetry を反映する（`adaptor/controller/desktop_lifecycle.rs:69-93`、`desktop.rs:9-30`）。
- ログイン項目: 希望の読み書きは監督の gateway の接続を使う（`releash-desktop/src/adaptor/gateway/login_item.rs:55-83`）。
- サーバの設定ファイルの直接読み: シェルは起動時に data dir の `releash.toml` をサーバを通さず読み、`start_minimized`（`desktop.rs:66-68`）とログイン項目の復元（`desktop.rs:77`）に使う。
- サーバを止める操作は画面に無い。TS は `request_application_quit` を呼んでいない（`src/generated` 以外で 0 件）。
- `restart_desktop` は TS から呼ばれていない。更新の後の起動し直しは `infrastructure/platform/desktop_restart::restart` を直接呼ぶ（`releash-desktop/src/adaptor/gateway/desktop_update.rs:35-37`）。
- サーバ側の子プロセス向けの仕組み: `--internal-daemon [DATA_DIR]`（`src/bin/backend.rs:14-16`）、env `RELEASH_DAEMON_PARENT_PIPE` があるときの stdin の EOF での終了（`src/infrastructure/process/parent_lifetime.rs:10-32`、`src/lib.rs:35`）、stdout の `releash-shutdown-complete`（`src/adaptor/controller/daemon.rs:94`）、`ServerInfo.launch_id` に env `RELEASH_DAEMON_LAUNCH_ID` を入れる（`src/adaptor/controller/api/client_service.rs:6-7`、`proto/client.proto:2598`）。
- 停止の RPC: `RequestApplicationQuit`（`proto/client.proto:129,1999,2557`）は Exit / Restart と code を受け、サーバはどちらも `StopRequest::Exit { code }` にしてその code で終了する（`src/adaptor/controller/client/application_lifecycle.rs:10-29`）。サーバは停止時に自分の発見ファイルを消す（`src/infrastructure/local_api/server.rs:163`）。
- 「居なければ隣の `releashd` を detached で起動する」処理はどこにも無い（`releash-sdk/src/lib.rs` のモジュールは compatibility・data_dir・descriptor・discovery だけ）。

# Scope / Non-goals

## Scope

- 画面の起動の流れを、発見 → 互換判定 → 居なければ起動 → 接続にする。
- 隣の `releashd` を detached で起動する処理を `releash-sdk` に作る（CLI の #1905 も同じものを使う）。
- 画面の監督（domain / usecase / gateway、監督の Tauri コマンド、状態の配信と表示、Rust 側の死活監視、launch_id による同一性の確認）を消す。
- Quit を画面だけを閉じる操作にする。トレイに「サーバを停止」を足す。
- 停止の RPC を `StopDaemon` に置き換え、`RequestApplicationQuit` と `ServerInfo.launch_id` を reserved にする。
- 画面の更新で動いているサーバを止めない。
- desktop 設定・ログイン項目の希望・`start_minimized` を、監督とサーバの設定ファイルの直接読みを通さずに受け取る。
- サーバ側の子プロセス向けの仕組み（`--internal-daemon`、stdin の EOF での終了、stdout の完了マーカー、`RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE`、`spawn_guard`・`terminate_descendants`）を消す。

## Non-goals

- CLI の `status`・`server start|stop|restart`、`DaemonInfo` の購読での配信（#1905）。
- CLI の配置の規則（#1907）。ディレクトリ構成（#1854）。
- hook の健全性の監視、hook の絶対パス化（#98）。
- ネイティブ UI（#78）。
- 画面の data dir の決め方（既定だけのまま）。
- #1203 の spec の改訂（closed の記録）。

# Requirements

- R-001: 画面を起動したとき、同じ data dir で互換のあるサーバが動いていれば、画面はそのサーバに接続し、新しいサーバを起動しない。
- R-002: 画面を起動したとき、サーバが動いていなければ、画面は同じディレクトリの `releashd` を起動して接続する。起動したサーバは画面から独立したプロセスとして動く。
- R-003: 画面がサーバを起動できなかったとき、画面はサーバのプロセスの終了と stderr の末尾を表示する。
- R-004: 画面を起動したとき、動いているサーバと互換が無ければ、画面は接続せず、サーバと画面のどちらが古いかと双方の release を表示する。サーバが古いときは、サーバを停止して起動し直す操作を出す。
- R-005: Quit（トレイ、メニュー・Cmd+Q などの OS の終了、失敗の窓の Quit）は画面だけを閉じ、サーバと動いている agent の Session は動き続ける。
- R-006: トレイの「サーバを停止」は、動いている agent の Session も止まることを確認してから、サーバを止める。
- R-007: 画面を開いている間にサーバが居なくなったとき、画面は「動いていない」と表示し、利用者の操作でサーバを起動できる。画面は自分からサーバを起動し直さず、サーバを強制終了しない。
- R-008: サーバを止める RPC は `StopDaemon` で、終了の種類も終了コードも受けず、サーバは終了コード 0 で止まる。`RequestApplicationQuit` と `ServerInfo.launch_id` は proto で番号と名前が reserved になる。
- R-009: 画面の更新は新しいバイナリをインストールして画面を起動し直すだけで、動いているサーバを止めない。
- R-010: 画面の close_to_tray・`start_minimized`・crash reporting・performance telemetry・ログイン項目の希望は、サーバから受け取った値に従う。画面はサーバの設定ファイルを直接読まない。
- R-011: サーバは、親プロセスとの pipe、stdout の完了マーカー、launch_id の env、`--internal-daemon` の引数を持たない。

# Assumptions

- なし
