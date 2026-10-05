# Context

- 要求の正本: ISSUE #1908「[02] `daemon` ドメインを作り、サーバの identity・版・受付・停止を 1 か所に集める」（https://github.com/siro33950/releash/issues/1908）。
- 所属: マイルストーン #100「03. サーバを画面から独立させ、CLI を整える」（https://github.com/siro33950/releash/milestone/100）。着手順の 2 番目で、先行は #1852（main acac85ef で完了）。
- 補助資料: gRPC Health Checking Protocol（https://github.com/grpc/grpc/blob/master/doc/health-checking.md）、#1904、#1905、`docs/architecture/`、`docs/glossary/DOMAIN.md`。
- 何を domain に置くかは、それがドメイン知識（規則・遷移・受理条件）か、仕組みや期限の値かで決める（`docs/architecture/DOMAIN.md` の「ビジネスロジック専念」「横断的関心事を持たない」）。serving status とその遷移、受付可否、停止要求の受理、identity・版・capability、記録と到達したサーバの同一性の判定、`DaemonInfo` は domain に置く。停止の deadline の値、port と token の所在、pid が生きているかの確認は domain の外に置く。
- 互換は proto の標準に従う（マイルストーン #100）。互換を壊す変更は proto package のメジャー版を上げて表し、同じメジャー版の中は追加だけで互換を保つ（Google AIP-180・AIP-185）。
- クライアントとサーバの両方が使う期限の値は、proto の service option に 1 か所で定義する（マイルストーン #99 の方針）。
- どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ（マイルストーン #100）。

# Outcome

対象者は、Releash のサーバ（daemon）を利用するクライアント（画面のシェル、CLI、hook、後続の #1904・#1905 で作る画面と CLI）と、Releash の開発者である。

今は、サーバの identity が 4 系統に分かれ、互いに結び付いていない。起動時の受付判定は本番で常に ready なので、起動失敗の状態とそのための画面・RPC には到達しない。受付判定は Connect のコマンド経路にしか掛からない。停止の意図を表す型は 3 つある。発見ファイルは待ち受けを始める前に書かれる。`GetServerInfo` は `launch_id` と `release` しか返さないため、クライアントはサーバの identity・版・状態を 1 つの値として得られない。

変更後は、サーバ自身を表す `daemon` ドメインが、identity・版・capability・serving status・受付可否・停止要求の受理を 1 か所で所有する。発見ファイルと `GetServerInfo` の応答は、その公開像（`DaemonInfo`）の投影になる。クライアントは `GetServerInfo` からサーバの identity・版・protocol・capability・serving status を得られる。発見ファイルがあれば、サーバは要求を受けられる状態にある。

# Current Behavior

確認した時点: main acac85ef。パスは `src-tauri/src/` を起点とする。

- `GetServerInfo` は `launch_id`（シェルが発行し env で渡す値）と `release` だけを返す（`adaptor/controller/api/client_service.rs:1-10`）。`ServerInfo` は field 2・3 を使い、1 と名前 `instance_id`、4 と名前 `desktop_settings` が reserved（`proto/client.proto:2760-2767`）。protocol version と capability に当たる値は、どこにも無い。
- サーバの identity は、`launch_id`（`adaptor/gateway/daemon_supervision.rs:165-169`）、サーバが発行する `instance_id`（`infrastructure/local_api/server.rs:48`）、発見ファイルの pid と起動時刻（`infrastructure/local_api/discovery.rs:18-24`）、子プロセスの pid の 4 系統に分かれている。
- 発見ファイルは `local-api.json`（master token。CLI と hook が読む）と `client-api.json`（client token。シェルが読む）の 2 つ。どちらも bind の時点で書かれ（`infrastructure/local_api/server.rs:45-82`、`adaptor/controller/daemon.rs:460-462`）、待ち受けの開始（`daemon.rs:598`）より前にある。消す経路は `server.rs` に 4 つある（`:77-82`、`:131-142`、`:153-161`、`:184-195`）。
- 発見の表現は 2 つある（infrastructure の `LocalApiDiscovery`、domain の `DiscoveryContent`。`domain/local_api_discovery/mod.rs:1-49`）。DTO から値への変換（`adaptor/gateway/local_api.rs:83-89`、`:237-245`）と評価の組み立て（CLI 用 `:69-117`、シェル用 `:221-265`）が 2 経路ある。
- 保存先を開けずに起動に失敗すると、`adaptor/controller/daemon.rs:92-104` が失敗を分類（`classify_startup_failure`、`:648-663`）してログに書き、`Err` を返す。`lib.rs:73-84` を通ってプロセスは終了コード 1 で終わり、stderr に「説明 (相関 ID)」が出る。発見ファイルは書かれない。画面側の監督は、この終了と stderr を起動失敗として表示する（`domain/daemon_supervision.rs:190-203`）。
- 起動の受付判定 `ApplicationStartupAuthority` は、本番で常に `ready()`（`daemon.rs:105-106`）。`failed` は `#[cfg(test)]` にだけある（`usecase/application_startup.rs:107-122`）。このため、購読 `startup-outcome`（`usecase/state_subscription/reads.rs:381`）は本番で常に Ready になる。画面の起動失敗画面（`src/App.tsx:29-80`、`:291-323`）と RPC `QuitAfterStartupFailure`（`proto/client.proto:2711`、`adaptor/controller/client/application_lifecycle.rs:12-20`）には到達しない。
- 受付判定が掛かるのは Connect のコマンド経路だけ（`adaptor/controller/api/client.rs:82`、`adaptor/controller/client/dispatch.rs:83-91`、`:183-192`）。購読などの RPC（`client_service.rs:1-91`）と HTTP local API は判定を通らない。停止要求の受理から API の停止までの間に届いた要求も受け付ける。
- 停止要求は wire の `ApplicationQuitIntentDtoV1`（Exit / Restart）で届く（`adaptor/presenter/application_lifecycle_v1.rs:52-57`）。domain の `ApplicationQuitIntent`（`domain/application_lifecycle.rs:3-7`）に写した後、Exit も Restart も exit code に畳まれる（`adaptor/gateway/application_lifecycle.rs:12-14`）。サーバは停止手順を実行した後、stdout に `releash-shutdown-complete` を出し、その exit code で終了する（`daemon.rs:68-74`）。最初の停止要求を受け取った `Daemon::wait` は channel を閉じる（`daemon.rs:70`）。そのため 2 回目以降の停止要求は `try_send` に失敗し、Connect のコード Internal のエラーになる（`adaptor/gateway/application_lifecycle.rs:15-19`、`adaptor/presenter/connect.rs:74-77`）。
- 停止手順（`usecase/application_lifecycle/mod.rs:13-38`）は、15 秒の `SHUTDOWN_TIMEOUT`（`domain/application_lifecycle.rs:1`）で打ち切る。ログ文言にも「15 second」が直書きされている（`:34`）。
- 停止手順の最初の段（`workflow_host.rs:1922-1923`）で、`CommandAdmission`（`domain/application_lifecycle.rs:27-44`）が止まる。それ以降、workflow の Command の開始と結果の取り込みは行われない（`adaptor/gateway/workflow/workflow_host.rs:1441-1442`、`:1687-1688`、`:1818-1819`、`:2110-2127`、`workflow_host/node_startup.rs:111-112`）。
- シェルの Tauri コマンドの受付が拒否するときは、`ApplicationUnavailable` を `{"type":"application_unavailable"}` として reject する（`adaptor/controller/command/mod.rs:31-49`、`:305-312`）。
- `docs/architecture/README.md:74-93` のドメイン一覧に `local_api_discovery` がある。`daemon` は無い。`docs/glossary/DOMAIN.md` に、サーバ自身に当たる語は無い。

# Scope / Non-goals

## Scope

- サーバ自身を表す `daemon` ドメイン（`Daemon` 集約、`DaemonInfo`、identity の値と同一性の判定、`StopRequest`）を作る。
- `GetServerInfo` の応答に、identity・版・protocol・capability・serving status を追加する。
- 発見ファイル 2 つを、Serving になってから書き、停止時に消す。発見の表現と評価の組み立てを、それぞれ 1 つにする。
- Daemon の受付可否を、Connect の全 RPC の入口に掛ける。workflow の Command の開始と結果の取り込みの判定も、同じ受付可否に置き換える。
- 停止の deadline の値を、domain から proto の service option へ移す。
- 本番で到達しない起動失敗の経路（購読 `startup-outcome`、`QuitAfterStartupFailure`、起動失敗画面）と、`CommandAdmission`、`ApplicationQuitIntent`、`usecase/application_startup.rs` の authority と失敗分類、controller の `classify_startup_failure`、`domain/local_api_discovery/` を削除する。
- domain の停止手順の trait から期限（`wait_for_deadline`）を外し、`ApplicationQuitIntentPort` を削除する。
- `docs/architecture/README.md` のドメイン一覧と、`docs/glossary/DOMAIN.md` の語彙を更新する。

## Non-goals

- 画面側の監督（`daemon_supervision` の domain / usecase / gateway）、`verify_identity`、`launch_id` の受け渡し、`--internal-daemon`、stdin EOF、stdout の完了マーカー（#1904）。
- wire の停止要求の形（`ApplicationQuitIntentDtoV1` の Exit / Restart）、`RequestApplicationQuit` の置き換え（#1904）。
- method ごとの scope、hook 用 token、Origin、buf（#1901）。HTTP local API の受付制御と HTTP の削除、CLI のクレート分け（#1902）。
- `releashd` への改名（#1903）。`StopDaemon` RPC、CLI の status / server コマンド、`DaemonInfo` の購読での配信（#1904・#1905）。
- クレートの分割（#1853）。
- `Compatibility`（互換の判定）。この ISSUE の範囲に本番の呼び出し元が無いため、最初に呼ぶ #1902 で作る。この ISSUE では、判定の材料になる protocol を `GetServerInfo` の応答に載せるところまで行う。
- 停止の deadline 以外の期限の値（`LOCAL_API_SHUTDOWN_TIMEOUT`、HTTP クライアントの期限、`desktop_restart` の期限、画面側の監督の期限）の移動。
- 既存の proto の enum（`enum Value { ... }` の形）の書き方。
- 発見ファイルの形（項目、ファイル名、権限）と、master token・client token の分離。
- `domain/application_lifecycle.rs` の `ApplicationShutdownGateway`（期限を除く停止の段）と `ApplicationLifecycleError` の置き場所と名前。

# Requirements

- R-001: `GetServerInfo` は、サーバの identity（サーバが発行する ID、pid、起動時刻）、release、protocol、capability の集合、serving status を返す。serving status は、要求を受けられる状態では Serving、停止要求を受理した後は Stopping である。`launch_id` と `release` は今と同じ値を返す。
- R-002: `GetServerInfo` が返す ID・pid・起動時刻は、同じサーバが書いた発見ファイルの `instance_id`・`pid`・`process_started_at` と一致する。
- R-003: `GetServerInfo` が返す protocol は、サーバがコンパイルした proto package のメジャー版（今は 1）である。capability の集合は空である。
- R-004: サーバは、要求を受けられる状態になってから、2 つの発見ファイル（`local-api.json` と `client-api.json`）を書く。停止時には両方を消す。片方だけが残る状態を作らない。発見ファイルの項目、ファイル名、権限、token の分離は今と同じである。
- R-005: Connect の RPC のうち、`GetServerInfo` と停止要求は serving status にかかわらず受け付ける。それ以外の RPC は、serving status が Serving のときだけ受け付ける。拒否したときのエラーの分類とコード（`APPLICATION_UNAVAILABLE`）は今と同じである。判定は新しく届く要求に掛け、すでに開いている stream と進行中の呼び出しは切らない。HTTP local API の受付は今と同じである。
- R-006: サーバが停止要求を受理した後は、workflow の Command を新しく開始せず、受理の後に始まる Command の結果の取り込みを行わない。受理の時点で進行中の取り込みは完了させる。停止手順は、進行中の取り込みが終わるのを待ってから Command を止める。
- R-007: 停止要求の Exit と Restart は、どちらもサーバの終了として扱う。サーバは、要求が運んだ exit code で終了する。
- R-008: サーバの停止手順は、停止の deadline（15 秒）を過ぎたら打ち切って終了する。deadline の値は proto の service option に 1 か所だけ定義され、サーバはその値を使う。
- R-009: 保存先を開けずに起動に失敗したサーバは、発見ファイルを書かずに終了する。失敗の説明と相関 ID は、今と同じ形でログと stderr に出る。
- R-010: 購読 `startup-outcome`、`QuitAfterStartupFailure`、画面の起動失敗画面を削除する。proto から消す項目は、field 番号と名前を reserved にする。画面は、監督が ready になるまで中身を描かない（今と同じ）。
- R-011: CLI とシェルが、発見ファイルからサーバへの接続先を確かめる判定（記録の妥当性、記録の pid と起動時刻が生きているプロセスと一致すること、到達したサーバが記録と同じであること）は、今と同じ結果を返す。
- R-012: シェルの Tauri コマンドの受付が拒否したときの reject の形 `{"type":"application_unavailable"}` は、今と同じである。OS の終了要求を受けたときの exit code とその後の挙動（監督が停止を進めて終了する）は、今と同じである。
- R-013: `docs/architecture/README.md` のドメイン一覧に `daemon` があり、`local_api_discovery` は無い。`docs/glossary/DOMAIN.md` に、Daemon、DaemonInfo、serving status、StopRequest の正規語がある。
- R-014: 停止要求を受理した後に届いた停止要求にも、成功（受理済み）を返す。サーバの終了コードは、最初に受理した停止要求の exit code である。

# Assumptions

なし
