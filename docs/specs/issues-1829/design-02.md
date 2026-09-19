# Design 02

## 開始状態

- 直前の Design は `docs/specs/issues-1829/design-01.md`。その「変える部分」は実装済みで、作業ツリーの未コミット変更として存在する。主要な状態は次のとおり。
  - `proto/client.proto` が `ClientService` を定義し、`AttachTerminalSurface` と `SubscribePush` を server streaming、その他を unary として公開する。
  - `src-tauri/Cargo.toml` が `connectrpc` 0.9.0（`axum` / `client` feature）を、`package.json` が `@connectrpc/connect` / `@connectrpc/connect-web` 2.2.0 を持つ。
  - クライアント向け Connect 入口は `src-tauri/src/adaptor/controller/api/client.rs` にあり、`/releash.client.v1.ClientService/{method}` へ `connectrpc::Router` の axum service を割り当てる。`auth.rs` の `require_client` が要求ごとの token 検証と Origin 検証（`tauri://localhost` / `http://tauri.localhost`）と CORS 応答を行う。
  - renderer 側は `src/lib/client.ts`（新設）が Connect transport を持つ。`src/lib/clientSocket.ts`、`src/generated/client_transport.ts`、`src/lib/desktopClientSocket.ts`、`src/components/ClientConnectionBanner.tsx` は削除済み。
  - サーバ側の `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs` は削除済み。
- 差分の基準は base ブランチ `main`、派生点 `00b57d77`。作業ブランチ `feat/issues/1829` は `020b097c`。
- この周までに解消・見送りとなった Thread はない。前の周で resolve した Thread、`[DEFERRED]` で人間へ渡した件、不成立として扱った件はいずれも 0 件である。
- open Thread は 18 件で、すべて `[FIX_POLICY]` を持つ。18 件が今周で変える部分である。
- Requirements・Behavior は今周で変更していない。R-001〜R-014 / R-016 / R-017 と B-001〜B-016 / B-018 / B-019 の対応は保たれており（R-015 / B-017 は欠番）、自動判断による修正は加えていない。

## 変える部分

- 終了済み terminal の stream 正常完了の扱い: `src/lib/client.ts:315-327` が stream の正常終了でも `onClosed` を呼び、終了済み terminal への再アタッチが反復する状態を、終了と切断を区別して正常完了を復旧の契機として扱わない形へ変える。根拠: Thread cc9a7c59、R-010 / R-011 / B-004 / B-013。ルート: 委任
- 世代不一致時の共有 stream 処理: `src/hooks/useTerminal.ts:543` が世代不一致で return する経路で `attached` を解決せず、全 attachment が共有する `streamProcessing` が停止する状態を、新しい attachment の snapshot と出力が処理される形へ変える。根拠: Thread 0003cc34、R-010 / R-011 / B-004 / B-013。ルート: 委任
- 生成コードの接続破棄: `scripts/generate-client-protocol.mjs:57` が出力する `ConnectError` 処理が `refreshClient()` を引数なしで呼び、旧世代の要求の失敗が確立済みの現行接続と進行中要求を破棄する状態を、世代の対応を守る形へ変える。根拠: Thread a061d437、R-010 / B-013。ルート: 委任
- Connect 経路の受信要求上限: `src-tauri/src/adaptor/controller/api/client.rs:198-200` が `connectrpc::Router` を既定（`DEFAULT_MAX_REQUEST_BODY_SIZE` / `DEFAULT_MAX_MESSAGE_SIZE` = 4 MiB）のまま `into_axum_service` し、変更前の 16 MiB から縮小している状態を、変更前に受理できた大きさが受理される形へ戻す。根拠: Thread b6c9cd32、R-011 / B-001。ルート: 委任
- `StopWatching` の応答: `api/client.rs:80-88` が watcher を drop するだけで停止の完了・失敗を確認せず成功を返す状態を、応答が実際の停止結果を反映する形へ変える。根拠: Thread e6c5edb8、R-011 / B-001。ルート: 委任
- watcher の所有と受理判定の配置: `api/client.rs:22` の購読ごとの watcher 登録簿、`:113-116` の「購読が無ければ not_found」という受理判定、`:164-192` の Drop による停止手順を controller が持つ状態を、内側の層が所有する形へ変える。根拠: Thread 959f5378、`docs/architecture/CONTROLLER.md`「業務ロジックを書かない」「受理判定を controller で書かない」、`docs/architecture/DOMAIN.md` の状態所有。ルート: 委任
- 監視開始の公開経路: `proto/client.proto:3832,3835` の `StartGitDirWatching` / `StartWatching` と `:3887-3888` の `WatchFiles` / `WatchGitDirectory` が監視開始を二系統で公開し、push 購読への束縛・監視枠の消費・購読終了時の解放の規則が経路によって食い違う状態を、公開経路を一つに集約した形へ変える。根拠: Thread f87033b6、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」、R-005 / B-007。ルート: 委任
- 発信 Connect client と共有型の配置: `src-tauri/src/adaptor/controller/api/protocol/connect.rs:29` が発信 call（`client_calls.rs`）を受信入口配下に include し、`src-tauri/src/adaptor/gateway/desktop_client.rs:1-4` がそれへ依存する状態を、`docs/architecture/CONTROLLER.md`「protocol/（メッセージ型）」と `GATEWAY.md`「外向き通知の送信」の責務境界に沿う配置へ変える。根拠: Thread f501162b。ルート: 委任
- daemon 同一性判定の所有: `src/lib/client.ts:74-76` が `GetServerInfo` の `launchId` 同一性を renderer で判定し、同じ規則が `usecase/daemon_supervision.rs` と `domain/daemon_supervision.rs` にも在る状態を、Rust の規則が所有し renderer は判定結果を受け取るだけの形へ変える。根拠: Thread 48444112、AGENTS.md「全てのアプリケーションロジックは Rust に置く。例外なし」、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」。ルート: 委任
- milestone 77 説明文の記述整合: 判断③・判断⑧が Connect へ改訂済みである一方、冒頭「デスクトップ UI を WebSocket クライアント化する」・土台「クライアント向け ws はこの上に新設する」・成果「daemon＋ws」が WebSocket 前提のまま残り、同一文書から相反する採用方針を読み取れる状態を解消する。根拠: Thread c81ba86e、R-014 / B-016。ルート: 委任。R-014 の対象範囲は判断③・判断⑧のまま拡張せず、固定するルート 11 が改訂先として固定した milestone 説明文内の記述整合に限る。
- 非同期 ack の失敗契約の検証: `src/hooks/useTerminal.test.ts:2052` が ack の失敗を同期 throw として与え、到達しない同期 catch に依存した検証になっている状態を、本番と同じ Promise rejection の契約で失敗し、その失敗から新しい attachment への再同期を観測できる形へ変える。根拠: Thread fd6bcb77、R-010 / R-011 / B-004 / B-013。ルート: 委任
- 消費者の無い `instance_id`: `api/client.rs:15,29` の `ClientApiDeps.instance_id` と、それを載せるだけの `client_service.rs:7` の `ServerInfo` フィールドが、廃止した operation registry を支えるためだけに残る状態を、削除範囲の一部として無くす。根拠: Thread e7421888、R-017 / B-019、固定するルート 8。ルート: 委任。discovery file の `instance_id` は対象外。
- 削除済みコマンドの mock 分岐: `tests/helpers/tauri-mock.ts:578-579` が削除済みの `list_client_handoff` / `forget_client_operation` を成功扱いする状態を、削除範囲の一部として無くし、テスト補助が受理するコマンド集合を実際の公開コマンド集合と一致させる。根拠: Thread ac0db313、R-017 / B-019、固定するルート 8。ルート: 委任
- terminal codec の変換検証: `src/lib/clientProtocol.test.ts:22` に resize（rows / cols）、入力不可、終了済み snapshot（終了状態と終了コード）の各変換分岐の検証がない状態を、入力と出力を検証できる形へ変える。根拠: Thread 461f8d72、R-003 / R-011 / B-004 / B-005。ルート: 委任
- `DetachTerminalSurface` の検証: `api/client.rs:90-98` の detach 分岐について、実 Connect 経路の RPC としての attachment 解放と backend への作用が、stream の drop とは別に検証されていない状態を、検証できる形へ変える。根拠: Thread 5df5add7、R-003 / R-011 / B-004 / B-005、`docs/architecture/TEST.md`。ルート: 委任
- daemon 同一性不一致の拒否の検証: `src/lib/client.desktop.test.ts:17` に、endpoint の同一性と接続先から取得した同一性が一致しない場合に業務 RPC と復元完了へ進まないことの検証がない状態を、検証できる形へ変える。根拠: Thread e05f1bb6、R-011 / R-013 / B-001 / B-015。ルート: 委任
- 起動中の送信前拒否の残存: `src-tauri/src/adaptor/controller/command/client.rs` の `get_client_endpoint` が `supervisor.attach` → `connection()` を呼び、`src-tauri/src/usecase/daemon_supervision.rs` の `connection()` が `connection_admitted()` false のとき「Daemon is not ready.」を返すため、起動中・切替中を理由とする送信前の拒否がこの経路に残る状態を、拒否しない形へ変える。検証は `src-tauri/src/usecase/daemon_supervision_test.rs:325` を含め、実際の要求経路で行えるようにする。根拠: Thread f320ceb6、R-016 / B-018、固定するルート 7。ルート: 委任
- ack credit 解放の検証: `src-tauri/src/adaptor/controller/api/client_stream_test.rs:385` に、`AckTerminalSurfaceOutput` unary を実 Connect 経路で送ったときの producer の credit 解放と配信再開の検証がない状態を、検証できる形へ変える。根拠: Thread 79699b85、R-003 / B-004、`docs/architecture/TEST.md`。ルート: 委任

## 固定するルート

今周で新たに固定する実装上の指定はない。design-01 で固定したルート 1〜11 を今周も維持する。解除するルートはない。

1. （design-01 維持）Rust サーバのトランスポートに `connectrpc` crate を使う。範囲: クライアント向け RPC のサーバ実装。粒度: 使用する crate の指定のみで、配置と構成は委任。
2. （design-01 維持）Web クライアントのトランスポートに `@connectrpc/connect-web` を使う。範囲: renderer のクライアント実装。粒度: 使用するパッケージの指定のみ。
3. （design-01 維持）自前 RPC 層として `src/lib/clientSocket.ts` と `src/generated/client_transport.ts` を削除する。範囲: この 2 ファイル。
4. （design-01 維持）desktop の Tauri IPC 通信経路を廃止する。対象は `src/lib/desktopClientSocket.ts` の `DesktopClientSocket` と、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client` / `admit_client_command`。
5. （design-01 維持）backend 状態の push を Connect の server streaming に載せる。粒度: トランスポート方式の指定のみ。
6. （design-01 維持）terminal を出力 = server streaming、入力 = unary に分解する。粒度: RPC 種別の指定のみ。入出力の対応付けとフロー制御の扱いは委任。
7. （design-01 維持）起動・切替中に変更要求を拒否する受付制御を廃止する。対象は `admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted`。
8. （design-01 維持）結果を確認できない変更要求を扱う仕組みを、クライアント側・サーバ側ともに廃止する。サーバ側の削除対象は `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs`。
9. （design-01 維持）`ClientConnectionBanner` を廃止する。範囲: `src/components/ClientConnectionBanner.tsx` と `src/App.tsx` の描画。
10. （design-01 維持）各 UI の `onUncertain` / `retryClientOperation` / `dismissClientOperation` を除去する。範囲: `CreateWorktreeModal` / `WorkspaceList` / `DeleteWorktreeDialog` / `SettingsModal` / `NodeContentView` / `useNotionSettings` / `useProviderAvailabilitySettings` / `useAppSettings` / `ProviderAvailabilitySettings`。
11. （design-01 維持）判断③・判断⑧の改訂先は、判断の正本である GitHub milestone 77 の説明文だけとする。範囲: milestone 説明文。

## 変えないもの

- Requirements・Behavior。今周は変更しない。検証不足を指摘された 5 件（Thread 79699b85 / e05f1bb6 / 5df5add7 / 461f8d72 / fd6bcb77）についても受入条件を追加しない。理由: 指摘の存在を理由に要求を増やさないため。
- R-014 の対象範囲。判断③・判断⑧のままとし、milestone 説明文全体の整合を新たな要求にしない。理由: 正本（Issue #1829）が対象としているのは判断③であり、対象範囲を広げないため。
- discovery file の `instance_id`。理由: 廃止した operation registry のための状態ではないため。
- daemon の起動・監視・停止とライフサイクル統合（Issue #1203）。`get_daemon_status` / `retry_daemon` / `quit_desktop` / `complete_desktop_restoration` / `fail_desktop_restoration` と `DaemonBoundary` の画面を含み、Tauri command のまま残す。起動・切替中の受付制御だけが廃止対象である。理由: #1203 の成果を変更しないため。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と、その認証。リクエスト単位の token 検証と Origin 検証を加える範囲は、クライアント向け RPC の入口に限る。理由: 本変更の対象外であり、既存の入口の振る舞いを変えないため。
- UI shell に残す Tauri の機能（dialog、opener、updater ＋ process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）と、起動結果の取得・起動失敗時の処理。理由: 通信基盤の置き換えと独立しているため。
- 既存 spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）。判断③・判断⑧を転記した箇所と `docs/specs/issues-1201/behavior.md` の B-009〜B-031 を含め、一切変更しない。理由: 終わった spec を改訂対象にしないため。
- milestone 77 の判断①（`.proto` が protocol の正）、判断④（ローカルは loopback ＋ token ファイル認証）、判断⑦（master token を renderer へ露出させない）。理由: 本変更が見直すのは判断③と判断⑧だけであるため。
- 対象はローカル loopback 上の接続だけとし、リモートアクセス経路は扱わない。理由: 本変更の対象外であるため。

## 未確定・リスク

- この周で Requirements・Behavior に自動判断による修正は加えていない。Assumptions に「自動判断: 未決」として残した要求はなく、`[DEFERRED]` で人間へ渡した件、不成立として扱った件もない。
- design-01 の「未確定・リスク」に挙げた 3 点（restoration 完了経路と削除対象の依存、WebView の origin から daemon への Connect 要求の成立、`connectrpc` crate の server streaming の充足）は、開始状態の実装がいずれも対応する経路を備えており（`get_client_endpoint` ＋ `complete_desktop_restoration`、`require_client` の Origin 検証と CORS 応答、`AttachTerminalSurface` / `SubscribePush` の server streaming）、これらを未充足とする open Thread も無いため、今周の未確定として再掲しない。
