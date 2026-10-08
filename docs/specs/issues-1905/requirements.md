# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1905 （[08] サーバの状態確認・起動・停止・再起動を CLI に足し、サーバの状態を配信する）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、#1903（`docs/specs/issues-1903/`）、#1904（`docs/specs/issues-1904/`）、#1908（`docs/specs/issues-1908/`）、#1944（[13] フッター）
- 本文の「今の作り」の file:line は main `4533af2f` 時点のもの。この文書は main `ebc04d12`（#1904 の後）で読み直した事実に基づく。
- 本文の `DaemonInfo` は、サーバの domain の値（`src-tauri/src/domain/daemon/mod.rs:34-41`）で、wire では `GetServerInfo` が返す `ServerInfo`（`proto/client.proto:2578-2592`）がその投影である。
- サーバの起動・停止・稼働確認の処理は、#1904 で `releash-sdk` に作られている（`src-tauri/releash-sdk/src/daemon.rs`）。#1904 の design で、CLI の `server stop` は画面の「サーバを停止」と同じ処理を使い、停止の待ちの上限は `shutdown_timeout_ms` と決まっている。
- マイルストーン #100 の規則: CLI は状態を持たず、実行ごとに発見 → 互換判定 → 呼び出しを行う。読み取りは購読の最初の状態を受け取って閉じる。`GetServerInfo` だけは接続確立と互換判定の呼び出しである。timeout・再試行・deadline の値は 1 か所に置く。
- `status` の表示は `GetServerInfo` の応答から作る。`status` は互換の無いサーバにも表示する必要があり、`GetServerInfo` は互換判定のための呼び出しとして読み取りの規則の例外になっているため。
- data dir と接続に要る情報（接続先と token の所在）は、CLI が解決した data dir から導く。発見ファイルは data dir の中にあり、見つかったサーバの data dir は CLI が解決したものと同じになるため。
- 「画面がある」かどうかは、CLI の実行ファイルの実体（symlink を辿った先）が `.app` の中にあるかで判定する。`.app` には画面・`releashd`・`releash` が同じ版の組として同梱され（`src-tauri/releash-desktop/tauri.conf.bundle.json:7-10`）、#1904 は隣の `releashd` を実行ファイルの位置で探すと決めているため。
- 対応プラットフォームは macOS（AGENTS.md「リリース」）。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-81 から受けた。

# Outcome

- 対象者: CLI を使う人と agent、サーバの発見・互換判定・起動に同梱の CLI を使うネイティブ UI（#78）、サーバの状態を画面に出す #1944、Releash の開発者。
- 現在の問題: CLI からサーバの状態を確かめる手段も、起動・停止・再起動する手段も無い。版が合わないときの CLI の案内は「どちらが古いか」だけで、どう直せばよいかを示さない。購読でサーバ自身の状態を受け取れない。CLI の接続の手順は `releash-sdk` の手順と別に書かれ、待ち時間が直書きされている。
- 変更後の状態: CLI の `status` でサーバの状態と互換を確かめられ、機械が読める形でも得られる。`server start|stop|restart` でサーバを起動・停止・再起動でき、`releash` だけで画面（またはサーバの状態）にたどり着ける。版が合わないときは、サーバの再起動かクライアントの更新を案内する。サーバの状態は購読でも配信され、CLI の表示と購読の値は同じサーバの状態を出どころにする。

# Current Behavior

main `ebc04d12` で読んで確かめた挙動。パスは `src-tauri/` 起点。

- CLI のコマンドは `workflow`・`review`・`hook`（隠し）・`completion` だけ（`releash/src/lib.rs:20-38`）。引数なしで実行すると clap がコマンドの指定を求めて終了コード 2 で終わる（`docs/guide/cli.md:18` も同じ記述）。
- CLI からサーバを起動・停止・確認する手段は無い。`releash-sdk` の `daemon::running`・`daemon::server_info`・`daemon::start`・`daemon::stop`（`releash-sdk/src/daemon.rs:50-145`）は、画面だけが使っている（`releash-desktop/src/adaptor/gateway/daemon_connection.rs:58-90`）。
- `GetServerInfo` は、サーバの `DaemonInfo` から daemon_id・pid・process_started_at・release・protocol・capabilities・serving_status を返す（`src/adaptor/controller/api/client_service.rs:1-8`、`src/adaptor/presenter/daemon.rs:1-19`）。データの場所は返さない。
- 購読の対象にサーバ自身の状態は無い（`src/usecase/state_subscription/target.rs:5-46`、`src/adaptor/presenter/state_subscription_target.rs:33-121`）。
- CLI の接続（`releash/src/client.rs:23-61`）は、発見ファイルを読み、プロセスを照合し、自前で Connect client を組み、`GetServerInfo` で同一性を確かめ、`Compatibility::assess` で互換を判定する。互換が無いと `failed_precondition` で `server is older` または `client is older` と双方の release を返す（`:45-59`）。サーバの再起動やクライアントの更新の案内は無い。RPC が `unimplemented` を返したときの案内も無い。
- hook は、発見ファイルの client token ではなく env `RELEASH_PROVIDER_LIFECYCLE_TOKEN` の token で接続する（`releash/src/commands.rs:525`）。`releash-sdk` の `daemon::client` は、発見ファイルの token を固定で使う（`releash-sdk/src/daemon.rs:37-48`）。
- 発見ファイルが無い・古い・壊れている・接続できないときは `unavailable`、終了コード 1（`docs/guide/cli.md:388-394`）。古い発見ファイルでは `client discovery is stale` を返す（`releash-sdk/src/discovery.rs:92-94`）。
- CLI は Connect の呼び出しの期限（`releash/src/client.rs:40`）と購読の最初の状態を待つ期限（`:68`）を 5 秒で直書きしている。proto の `ClientService` の service option の `default_timeout_ms` は 120000（`proto/client.proto:2486`）で、`releash-sdk` の `daemon::client`（`releash-sdk/src/daemon.rs:37-48`）と画面はこの値を使う。

# Scope / Non-goals

## Scope

- CLI に `status [--json]`、`server start`、`server stop`、`server restart` を足す。
- CLI の引数なしの実行を、サーバを起動して画面を開く（画面が無ければ状態を表示する）操作にする。
- サーバの状態（`DaemonInfo`）を購読の対象として配信する。
- 版が合わないとき・古いサーバが RPC を知らないときの CLI の案内を足す。
- CLI の既存コマンドの接続の手順を `releash-sdk` の手順に揃え、CLI の待ち時間の直書きを proto の `default_timeout_ms` に揃える。
- `docs/guide/cli.md` を、足したコマンドと変わった挙動に合わせる。

## Non-goals

- 画面へのサーバの状態の表示（#1944）。
- `ServerInfo` と `DaemonInfo` へのデータの場所の追加。
- hook がサーバを起動すること（マイルストーン #100 の方針で、サーバが居ないときに hook が受けた信号は捨てる）。
- CLI の配置の規則（#1907）。hook の絶対パス化（#98 の #1906）。
- launchd の plist・systemd の unit の提供。

# Requirements

- R-001: `releash status` は、クライアントの release と protocol、data dir、サーバが動いているか、動いていればサーバの identity（daemon_id・pid・起動時刻）・release・protocol・capability・serving status・起動からの時間、互換の判定（互換あり／サーバが古い／クライアントが古い）を表示する。サーバの情報は `GetServerInfo` の応答から作る。
- R-002: `releash status` は、サーバが動いていないときは「動いていない」と表示して終了コード 0 で終わる。互換の無いサーバが動いているときも、その情報と互換の判定を表示して終了コード 0 で終わる。
- R-003: `releash status --json` は R-001 の項目を JSON で出し、接続に要る情報として接続先（host と port）と token の所在（発見ファイルのパス）を含める。token の値そのものは出さない。項目を足すときは、既存の項目の名前と意味を変えない。
- R-004: `releash server start` は、サーバが動いていなければ CLI と同じディレクトリの `releashd` を、CLI から独立したプロセスとして起動し、起動を確かめて終了コード 0 で終わる。すでに動いていれば何もせず終了コード 0 で終わる。起動できなかったときは、プロセスの終了と stderr の末尾を表示して終了コード 1 で終わる。
- R-005: `releash server stop` は、サーバが動いていれば停止を要求し、プロセスの終了と発見ファイルの消失を確かめて終了コード 0 で終わる。待つ上限は proto の `shutdown_timeout_ms` で、上限までに確かめられなければ終了コード 1 で終わる。サーバが動いていなければ終了コード 1 で終わる。
- R-006: `releash server restart` は、サーバが動いていれば停止してから起動し、動いていなければ起動だけを行う。どちらの場合も、終わった時点で CLI と同じディレクトリの `releashd` のサーバが動いている。出力は、停止してから起動したのか、動いていなかったので起動したのかを区別する。
- R-007: `releash`（引数なし）は、サーバが動いていなければ R-004 と同じに起動する。CLI の実行ファイルの実体（symlink を辿った先）が `.app` の中にあり、CLI が解決した data dir が画面の使う既定の data dir と同じであれば、その `.app` を開く。`.app` の中に無ければ、`releash status` と同じ表示を出す。`.app` の中にあっても data dir が画面の既定と違うときは、`.app` を開かず、data dir が画面の既定と違うために開かなかったことと、`releash status` と同じ表示を出す。
- R-008: サーバの状態（`DaemonInfo`）を購読の対象として配信する。購読の値と `GetServerInfo` の応答は、サーバの同じ `DaemonInfo` から作られ、同じ項目を持つ。serving status が変わると、購読している client に変わった値が届く。
- R-009: CLI の workflow・review のコマンドは、サーバが古いと判定したとき、`releash server restart` で更新できることを案内する。クライアントが古いと判定したときは、クライアントの更新を案内する。どちらも `failed_precondition`、終了コード 1 のままである。`releash status` も、サーバが古いときは `releash server restart` を、クライアントが古いときはクライアントの更新を案内する。
- R-010: CLI のコマンドの呼び出しにサーバが `unimplemented` を返したとき、CLI は `releash server restart` で更新できることを案内する。
- R-011: CLI の既存コマンドと足したコマンドは、サーバを同じ手順で発見・照合する。発見ファイルが古いときは、サーバが動いていないときと同じ扱い（既存コマンドは `unavailable`、終了コード 1）になる。
- R-012: CLI の Connect の呼び出しの期限と、購読の最初の状態を待つ期限は、proto の `default_timeout_ms` である。CLI に期限の値の直書きは無い。
- R-013: `docs/guide/cli.md` は、足したコマンド、引数なしの実行、版の案内、サーバ未起動時の挙動を、変更後の挙動どおりに説明する。

# Assumptions

- なし
