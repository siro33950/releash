# Context

- 要求の正本: Issue #1829「クライアント通信を Connect へ置き換え、自前 RPC 層と通信状態の UI 表現を廃止する」、milestone 77「01. ローカル server-client 化（基盤）」。
- 背景資料: Issue #1199 / #1200 / #1201 / #1202 / #1203、`src/lib/clientSocket.ts`、`src/lib/desktopClientSocket.ts`、`src/generated/client_transport.ts`、`proto/client.proto`、`src-tauri/src/adaptor/controller/api/auth.rs`、`src-tauri/src/adaptor/controller/command/client.rs`、`src/components/ClientConnectionBanner.tsx`、https://buf.build/blog/connect-rust-joins-the-connect-project 、https://connectrpc.com/docs/go/streaming/ 、https://connectrpc.com/docs/web/interceptors/ 。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: `.proto` が protocol の正であり、Rust / client の型はそこから生成する。
  - 判断④: ローカルは loopback＋token ファイル認証とする。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行し、master token を renderer へ露出させない。
- milestone 77 の設計判断のうち、本変更が見直すもの。判断の正本は GitHub milestone 77 の説明文である。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。これを Connect へ置き換える。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。terminal を独立した RPC に分解するため、これを見直す。
- 正本が確定させている採用方針。
  - Rust サーバは `connectrpc` crate を使う。local API は既に axum であり、Tower / Axum 統合の上に載せる。1 サーバで Connect / gRPC / gRPC-Web を話す。
  - Web クライアントは `@connectrpc/connect-web` を使う。
  - push は server streaming に載せる。
  - terminal は「出力 = server streaming、入力 = unary」に分解する。ブラウザから bidi / client streaming は使えない（fetch がリクエストボディをストリーミングできない）。
- desktop の WebView の origin は、macOS / Linux が `tauri://localhost`、Windows が `http://tauri.localhost` である。
- Issue #1202（Tauri 非依存の headless デーモン抽出）は closed であり、daemon は desktop とは別のプロセスで動作する。Issue #1203（launchd 常駐）は open であり、UI と daemon のライフサイクル統合はコミット `6b3a9e27` として本変更の派生元に含まれる。

# Outcome

- 対象者は、iOS / Android / Windows / macOS / Linux のクライアントを追加する開発者と、desktop 利用者である。
- 現在、クライアント通信は WebSocket の上の自前 RPC 層で成り立っている。この層は `proto` から生成されないため、クライアントを増やすたびに各言語で再実装することになる。認証は WebSocket の handshake で 1 回だけ行われ、token を失効させても接続が生きている限り要求が通る。Origin は検証されない。desktop は WebSocket を使わず Tauri IPC で daemon へ frame を中継しており、この経路は他プラットフォームのクライアントで再現できない。加えて、通信の内部状態（未送信・結果不明・再接続中）が画面に提示され、利用者が通信路の事情を判断させられている。
- 変更後は、desktop を含むすべてのクライアントが Connect でサーバの RPC を呼び、トランスポート層はサーバ・クライアントとも既製の実装になる。新しいプラットフォームのクライアントは、生成されたクライアントコードと transport アダプタだけで同じサーバを同じ手順で呼べる。認証はリクエスト単位で行われ、token の失効が次のリクエストで効き、Origin も検証される。通信の内部状態は利用者へ提示されず、接続断は再接続と状態の再取得で回復する。

# Current Behavior

最初の周の開始時点（`feat/issues/1829`、`020b097c`）で、コードを読んで確認した挙動。アプリケーションの起動・テストの実行による確認は行っていない。

## トランスポートと protocol 定義

- daemon の local API（axum、127.0.0.1 bind）が `/v1/client` を WebSocket route として公開する（`adaptor/protocol/client.rs`、`adaptor/controller/api/client.rs`）。frame 上限は 16 MiB、同時接続は 16、同時処理中の要求は 64 に制限される。
- `proto/client.proto`（3,783 行）は service を定義しない。`CommandRequest` の単一 `oneof command`、`Push` の単一 `oneof event`、および 12 種の body を持つ `Envelope`（`request` / `response` / `push` / `stream` / `ack` / `request_ack` / `push_resync` / `stream_closed` / `hello` / `heartbeat` / `operation_query` / `operation_status`）を定義する。
- terminal は `Envelope` の `stream` として同じ接続に載る（`adaptor/controller/api/client_stream.rs`）。フロー制御は `attachment_id`＋`sequence` / `ack` で行う（`proto/client.proto:3511-3554`）。
- 型生成は `pnpm generate:protocol`（protoc ＋ `@bufbuild/protoc-gen-es` ＋ `scripts/generate-client-protocol.mjs`）と `src-tauri/build.rs`（prost ＋ descriptor）で行う。`package.json` に `@connectrpc/connect-web` はなく、`src-tauri/Cargo.toml` に `connectrpc` crate はない。

## 自前 RPC 層

- `src/lib/clientSocket.ts`（1,049 行）が、`request_id` 相関、コマンド別 deadline、未送信の再送、heartbeat とタイムアウト検知、再接続、fingerprint による重複防止、`operationQuery` の状態機械（ready / bound / not_sent / disconnected / unknown / restored_unknown）、`orderingTarget` による順序保証、watch の再登録、stream の ack を実装する。
- `src/generated/client_transport.ts`（1,586 行）が、コマンド別の recovery / deadline / disconnect 方針を保持する。`scripts/generate-client-protocol.mjs` が生成する。
- サーバ側には、結果を確認できない変更要求を扱う実装がある。`src-tauri/src/domain/client_operation/`（`registry.rs` 407 行、`policy.rs` 258 行、`transmission.rs` 125 行、`handoff.rs` 60 行）が操作の識別・重複判定・結果の保持と破棄を持ち、`src-tauri/src/usecase/client_operation.rs`（135 行）と `src-tauri/src/adaptor/controller/api/client_operation.rs`（132 行）がそれを呼ぶ。

## desktop の通信経路

- `src/lib/clientSocket.ts:27` が `DesktopClientSocket` を `WebSocket` として import する。実体は Tauri の `invoke` と `Channel` である（`src/lib/desktopClientSocket.ts`）。
- `DesktopClientSocket` は `attach_desktop_client` で `DaemonSupervisionUsecase` へ attach し、`send_desktop_client_frame` で frame を送り、`detach_desktop_client` で切る。
- `src/lib/clientSocket.ts:838` が、すべての要求の送信前に `invoke("admit_client_command", { command })` を 1 往復する。拒否された要求は `not_sent` として扱われる。
- UI shell の Rust（`adaptor/gateway/daemon_supervision.rs`）が `tokio_tungstenite` で daemon の `/v1/client` へ WebSocket 接続し、renderer の frame を中継する。renderer が daemon へ直接接続する経路はない。
- `get_client_endpoint` を呼ぶ本番コードはない（テストにのみ残る）。
- `attach_desktop_client` / `send_desktop_client_frame` / `admit_client_command` / `detach_desktop_client` / `complete_desktop_restoration` / `fail_desktop_restoration` は、コミット `6b3a9e27`「feat(desktop): UIとdaemonのライフサイクルを統合する」で追加された。

## 起動・切替中の変更要求の受付

- `DaemonSupervisionUsecase::admit_client_command`（`src-tauri/src/usecase/daemon_supervision.rs:215`）は、daemon へ接続済みでなく、または `Supervision::client_command_admitted`（`src-tauri/src/domain/daemon_supervision.rs:483`）が false の場合に「Releash is starting or switching; this request was not accepted.」を返す。admitted となるのは `phase` が `Ready` のとき、`Restoring` で `RestoreState` / `Recovery` のとき、停止中で `Shutdown` のときに限られる。
- `src/components/DaemonBoundary.tsx:105` は `phase` が `ready` でない間、画面全体を覆い、描画した子を `inert` にする。利用者が起動・切替中に操作を発生させることはできない。

## 認証・認可

- `require_bearer`（`adaptor/controller/api/auth.rs`）が local API 全体に適用され、`Authorization: Bearer <token>` か、WebSocket handshake に限り `Sec-WebSocket-Protocol: releash-bearer.<token>` を受理する。token の一致判定は定数時間比較で行う。
- Origin header を検証する処理はない。CORS layer はない。
- WebSocket は handshake 時に 1 回だけ認証され、確立後の frame は再認証されない。
- discovery file は 2 つある。daemon の `local-api.json` に master token が書かれ、クライアント用 `client-api.json` に非 master token が書かれる。UI shell の Rust が `client-api.json` を読んで daemon へ接続する。renderer はいずれの token も受け取らない。

## 通信状態の UI

- `src/App.tsx:321` が `ClientConnectionBanner` を描画する。banner は接続状態の文言と、pending の要求の一覧を表示する。要求ごとに「接続の回復を待っています（未送信）。」「要求は送信されていません（未実行）。」「操作結果を確認できません。」のいずれかと、「元の操作の結果を確認」「確認済み」ボタンを出す。
- `onUncertain` は `clientSocket.ts` の要求オプションであり、`src/components/workspace/CreateWorktreeModal.tsx`、`src/components/workspace/WorkspaceList.tsx`、`src/components/workspace/DeleteWorktreeDialog.tsx`、`src/components/panels/SettingsModal.tsx`、`src/components/panels/NodeContentView/NodeContentView.tsx`、`src/hooks/useNotionSettings.ts`、`src/hooks/useProviderAvailabilitySettings.ts`、`src/hooks/useAppSettings.ts` が渡す。
- `retryClientOperation` は `ClientConnectionBanner.tsx`、`src/components/panels/ProviderAvailabilitySettings.tsx`、`SettingsModal.tsx`、`NodeContentView.tsx` が呼ぶ。
- `dismissClientOperation` は `ClientConnectionBanner.tsx` だけが呼ぶ。表示を消すだけで、再送も復旧もしない。

## daemon 監督

- `src/components/DaemonBoundary.tsx` が 250 ms 間隔で `get_daemon_status` を Tauri invoke し、`phase` が `ready` でない間は画面全体を覆う。`retry_daemon` / `quit_desktop` / `fail_desktop_restoration` を Tauri invoke で呼ぶ。
- restoration の完了は `completeClientRestoration`（`clientSocket.ts:1048`）から `DesktopClientSocket.completeRestoration` を経て `complete_desktop_restoration`（`launch_id` と `attachment_id` を伴う）へ渡る。

# Scope / Non-goals

## 変更するもの

- クライアント通信のトランスポート。WebSocket 単一トランスポートから Connect（サーバは `connectrpc` crate、Web クライアントは `@connectrpc/connect-web`）へ置き換える。
- `proto/client.proto` の、Connect の service と method としての定義。
- 自前 RPC 層（`src/lib/clientSocket.ts`、`src/generated/client_transport.ts`）の削除。
- desktop の Tauri IPC 通信経路（`DesktopClientSocket`、`attach_desktop_client` / `send_desktop_client_frame` / `detach_desktop_client`）の廃止。
- 起動・切替中の変更要求の受付制御（`admit_client_command`、`DaemonSupervisionUsecase::admit_client_command`、`Supervision::client_command_admitted`）の廃止。
- 結果を確認できない変更要求を扱う仕組みの廃止。クライアント側の状態機械と自動再送に加え、サーバ側の `src-tauri/src/domain/client_operation/`、`src-tauri/src/usecase/client_operation.rs`、`src-tauri/src/adaptor/controller/api/client_operation.rs` を含む。
- backend 状態の push の server streaming への移行。
- terminal の「出力 = server streaming、入力 = unary」への分解。
- リクエスト単位の token 検証と、クライアント向け RPC の Origin 検証。
- 通信の内部状態を提示する UI（`ClientConnectionBanner`、各 UI の `onUncertain` / `retryClientOperation` / `dismissClientOperation`）の撤去。
- milestone 77 の判断③と判断⑧の改訂。
- 要求の正本である Issue #1829 の本文のうち、`docs/specs/issues-1201/behavior.md` の B-009〜B-031 の改訂を求める記述の取り下げ。

## 変更しないもの

- daemon の起動・監視・停止とライフサイクル統合（Issue #1203）。`get_daemon_status` / `retry_daemon` / `quit_desktop` / `complete_desktop_restoration` / `fail_desktop_restoration` と `DaemonBoundary` の画面を含む。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と、その認証。
- UI shell に残す Tauri の機能（dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）、および起動結果の取得と起動失敗時の処理。
- 判断①（`.proto` が protocol の正）、判断④（loopback＋token ファイル認証）、判断⑦（master token を renderer へ露出させない）。
- 既存 spec（`docs/specs/issues-1199/`、`docs/specs/issues-1200/`、`docs/specs/issues-1201/`、`docs/specs/issues-1202/`、`docs/specs/issues-1203/`）の内容。milestone 77 の判断を転記した箇所と、`docs/specs/issues-1201/behavior.md` の B-009〜B-031 を含む。
- リモートアクセス経路。ローカル loopback 上の接続だけを対象とする。

# Requirements

- R-001: desktop を含むクライアントは、Connect プロトコルでサーバの RPC を呼ぶ。desktop が backend を呼ぶ通信は Tauri IPC を経由しない。
- R-002: backend 状態の push は、Connect の server streaming でクライアントへ配信され、画面へ反映される。
- R-003: terminal の出力は Connect の server streaming で配信され、terminal への入力は unary の RPC で送られる。
- R-004: `proto/client.proto` から生成されないクライアント側の RPC 層（`src/lib/clientSocket.ts`、`src/generated/client_transport.ts`）が存在しない。
- R-005: 新しいプラットフォームのクライアントは、`proto/client.proto` から生成したクライアントコードと transport アダプタだけで、desktop と同じ RPC を同じ手順で呼べる。
- R-006: サーバは要求ごとに token を検証する。token を失効させた後に送られる次の要求は、接続の確立時点で有効だった場合でも拒否される。
- R-007: サーバはクライアント向け RPC の Origin を検証し、許可しない Origin からの要求を拒否する。desktop の WebView の origin（macOS / Linux は `tauri://localhost`、Windows は `http://tauri.localhost`）は許可される。
- R-008: renderer は master token を受け取らない。master token は daemon の discovery file にだけ書かれる。
- R-009: 通信の内部状態（未送信・結果不明・再接続中）を利用者へ提示する UI が存在しない。
- R-010: サーバとの接続が切れた場合、クライアントは再接続する。接続の回復後に、状態の再取得、push の購読の復旧、terminal の再同期が行われ、現在の状態が画面へ反映される。
- R-011: desktop の機能は、変更前と同じ結果が画面へ反映される。terminal の入出力と、backend 状態の push による画面更新が動作する。
- R-012: CLI と provider hook が使う HTTP local API は、変更前と同じ経路・同じ認証で動作する。
- R-013: daemon の起動・監視・停止と `DaemonBoundary` の画面は、変更前と同じ振る舞いである。
- R-014: milestone 77 の判断③と判断⑧が、Connect を採る内容へ改訂されている。
- R-016: クライアントの変更要求は、daemon が起動中または切替中であることを理由に送信前に拒否されない。
- R-017: 応答が返らなかった変更要求は、自動で再送されない。その要求の結果を照会する仕組みと、結果の受け取りが確認されていない変更要求の記録を保持する仕組みは存在しない。
- R-018: 要求の正本である Issue #1829 の本文は、`docs/specs/issues-1201/behavior.md` の B-009〜B-031 の改訂を求めない。

# Assumptions / Open Questions

なし。
