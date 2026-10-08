# Design

パスは、`proto/`・`docs/` で始まるもの以外は `src-tauri/` 起点。行番号は main `ebc04d12` 時点。

## 変える部分

- CLI の接続の手順: `releash/src/client.rs:23-61` の `connect` を、`releash-sdk` の `daemon::running`（`releash-sdk/src/daemon.rs:62-72`）→ `daemon::client`（`:37-48`）→ `daemon::server_info`（`:50-60`）→ `Compatibility::assess`（`releash-sdk/src/compatibility.rs:9-15`）の順に呼ぶ形にし、自前の Connect client の組み立て（`client.rs:26-41`）を消す。`running` が「動いていない」を返したときは `unavailable` にする。SDK の `daemon::client` と `daemon::server_info` は、接続先（発見ファイル）と bearer token を分けて受け取る形にする。画面と CLI のコマンドは発見ファイルの token を、hook は env `RELEASH_PROVIDER_LIFECYCLE_TOKEN` の token（今の `releash/src/commands.rs:525`）を渡す。根拠: R-011、B-021。ルート: SDK に CLI のためだけの分岐を足さない。CLI が SDK の処理を順に呼ぶ。token を分けて受け取るのは、token を持つ主体（operator と hook）が複数あることの表現であり、CLI 専用の分岐ではない。hook の接続（`GetServerInfo` を含む。`proto/client.proto:2575` で SCOPE_HOOK でも呼べる）は env の token で行い、今と同じに動く。
- CLI の期限: `releash/src/client.rs:40` の呼び出しの期限は `daemon::client` の設定（`default_timeout_ms`）に任せ、`:68` の購読の最初の状態を待つ期限は `daemon::timeout("default_timeout_ms")`（`releash-sdk/src/daemon.rs:28-35`）で決める。根拠: R-012。ルート: CLI に option を読む処理を別に書かない。
- 版の案内: 互換の判定の結果から、サーバが古いときは `releash server restart` を、クライアントが古いときはクライアントの更新を案内する文言を作り、既存コマンドの `failed_precondition`（今の `client.rs:45-59`）と `status` の表示で同じものを使う。コマンドの呼び出しが `unimplemented` を返したときは、`releash server restart` を案内する。根拠: R-009、R-010、B-003、B-004、B-018〜B-020。ルート: 委任。
- `status [--json]`: `running` と `server_info` の結果、`Compatibility::assess` の判定、CLI が解決した data dir（`releash/src/lib.rs:83`）から表示を作る。起動からの時間は `process_started_at` から CLI が計算する。`--json` には接続先（host と port）と発見ファイル（`client-api.json`）のパスを含め、token の値は含めない。根拠: R-001〜R-003、B-001〜B-005。ルート: サーバの情報は `GetServerInfo` の応答から作る（購読は使わない）。data dir、接続先、発見ファイルのパスは CLI が解決した data dir から導き、`ServerInfo` と `DaemonInfo` に項目を足さない。CLI の中に `DaemonInfo` の別の形を持たない。JSON の項目名は委任。
- `server start|stop|restart`: `start` は `running` で動いているかを確かめ、動いていなければ `daemon::start`（`releash-sdk/src/daemon.rs:74-124`）で CLI の隣の `releashd` を起動する。`stop` は `running` で見つけたサーバに `daemon::stop`（`:126-145`）を使い、見つからなければ終了コード 1 にする。`restart` は、動いていれば `stop` の後に `start`、動いていなければ `start` だけを行い、出力でどちらだったかを分ける。根拠: R-004〜R-006、B-006〜B-013。ルート: 起動と停止の処理は `releash-sdk` の `daemon::start`・`daemon::stop` を使い、CLI に別の処理を書かない（#1904 の design で、トレイの停止・失敗の窓・CLI の `server stop` が同じ処理を使うと決まっている）。`restart` は stop の再利用ではなく、「終わった時点で今の版のサーバが動いている」ことの保証として定義する。
- 引数なしの実行: サーバが動いていなければ `server start` と同じ処理で起動し、CLI の実行ファイルの実体が `.app` の中にあればその `.app` を開き、無ければ `status` と同じ表示を出す。根拠: R-007、B-014、B-015。ルート: 実体は `current_exe` を canonicalize して求める。判定は実行ファイルの位置だけで行い、OS やディスプレイの有無は見ない。`.app` を開く処理は macOS 固有のため SDK ではなく CLI に置き、`cfg(target_os = "macos")` で区切る。
- サーバの状態の購読: 購読の対象に `DaemonInfo` を足し、serving status が変わったときに購読している client へ配信する。payload は proto の `StatePayload` の oneof に足す。根拠: R-008、B-016、B-017。ルート: 購読の payload と `GetServerInfo` の応答は、どちらもサーバ側の同じ `DaemonInfo`（`usecase/daemon.rs` が repository から得る値）から作る。対象の名前と payload の message の形は委任。
- CLI のガイド: `docs/guide/cli.md` の「実行」（`:16-21`）、「コマンド一覧」（`:28-44`）、「終了コード」（`:375-386`）、「サーバ未起動時の挙動」（`:388-394`）を、足したコマンド、引数なしの実行、版の案内、古い発見ファイルの扱いに合わせ、`status`・`server` の節を足す。根拠: R-013。ルート: 委任。

## 固定するルート

- `status` の情報は `GetServerInfo` の応答から作り、購読は使わない。購読の対象 `DaemonInfo` はこの ISSUE で作るが、使うのは #1944 から。
- 購読の payload と `GetServerInfo` の応答は、サーバ側の同じ `DaemonInfo` から作る。CLI の中に `DaemonInfo` の別の形を持たない。
- data dir、接続先、発見ファイルのパスは CLI が解決した data dir から導く。`ServerInfo`・`DaemonInfo` に data dir を足さない。
- 画面があるかの判定は、CLI の実行ファイルの実体（canonicalize した `current_exe`）が `.app` の中にあるかだけで行う。`.app` を開く処理は CLI に置き、`cfg(target_os = "macos")` で区切る。
- `restart` は、動いていなければ起動だけを行って成功にする。
- 起動・停止・発見・照合は `releash-sdk` の `daemon::{running, client, server_info, start, stop}` を CLI から順に呼び、SDK に CLI 専用の分岐を足さない。`daemon::client`・`daemon::server_info` は接続先と bearer token を分けて受け取り、hook は env の token を渡す。
- CLI の期限は `default_timeout_ms` だけを使い、`daemon::timeout` で読む。

## 変えないもの

- なし

## 未確定・リスク

- なし
