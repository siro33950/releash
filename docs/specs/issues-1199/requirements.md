# Context

- 要求の正本: Issue #1199「A1: walking skeleton（汎用エンベロープ＋dispatch＋push 1本・in-process ws・レイテンシ実測）」、milestone 77「02. ローカル server-client 化（基盤）」。
- 背景資料: Issue #1198、PR #1338、`AGENTS.md`、`docs/specs/issues-1198/`。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: req/resp は汎用エンベロープ（`request_id`）を経由して usecase 共有 dispatch へ渡す。push は typed を維持する。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。
  - 判断④: ローカルは loopback＋token ファイル認証とする。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行する。権限が同等でも識別子を分け、master token を renderer へ露出させない。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。
  - 判断⑥: strangler 移行（A0 掃除 → A1 walking skeleton → ドメイン被覆 → 既定切替 → デーモン抽出 → 常駐）。本変更は A1 にあたり、最初の GO/NO-GO ゲートである。
- 土台として確定している事項。
  - クライアント向け ws route は、MS82 で新設した local API（`adaptor/controller/api/`、axum、127.0.0.1 bind、discovery file）の上に新設する。
  - HTTP local API（workflow / provider-lifecycle）は CLI と provider hook のローカル専用入口として残し、クライアントの経路にはしない。
  - A0 で温存した旧 ws shell は PR #1338 で削除済みであり、再利用しない。
  - 既存の `/v1/terminal` WebSocket をエンベロープの雛形にする。
  - 認証方式は既存 terminal と同じ `Sec-WebSocket-Protocol` bearer 方式にする。

# Outcome

- 対象者は、A1 以降（ドメイン被覆・既定切替・デーモン抽出）を実装する開発者と、振る舞いが変わらないことを必要とする desktop 利用者である。
- 現在、desktop の req/resp は Tauri invoke、backend 状態の push は Tauri emit に固定されている。ws クライアントが同じ usecase を呼び、同じ状態変化を受け取るための共通の骨格（汎用エンベロープ、transport 非依存 dispatch、単一 push sink、クライアント token）が存在しない。またローカル ws の往復レイテンシが未測定であり、後続で Tauri 経路を ws へ寄せる（A-flip）判断の材料がない。
- 変更後は、クライアント向け ws route の上に汎用エンベロープと transport 非依存 dispatch と単一 push sink が通っており、req/resp 1 本と push 1 本が実際に ws 経由で desktop に届く。ローカル ws の往復レイテンシ実測値が記録され、GO/NO-GO 判断に使える状態になる。あわせて、実体のない `[server]` 設定が消える。

# Current Behavior

最初の周の開始時点（`feat/issues/1199`、`8f6a107a`）で確認した挙動。

## local API と ws

- local API は `adaptor/controller/api/` にあり、`127.0.0.1` の動的ポートへ bind する。discovery file に port と master token を書き出す。
- route は workflow HTTP、provider-lifecycle HTTP、`/v1/terminal` WebSocket の 3 系統である。クライアント向けの汎用 ws route は存在しない。
- 旧 ws shell（`ws_server` / `ws_bridge` / `WsBroadcaster`）はコードベースに存在しない。
- local API の bind に失敗した場合、Tauri の setup がエラーになり main window は作られない（`src-tauri/src/lib.rs`）。local API が起動していない状態で desktop の画面が表示されることはない。

## 既存 `/v1/terminal` エンベロープ

- 接続直後の要求は `AttachSurface { id, owner, attachment_id }` の 1 種類。応答は `status` タグ付きで `attached { id }` / `error { id, error { code, message } }` / `event { id, item }`。要求の `id` が応答に写される。
- attach 確立後の要求は `Write { owner, attachment_id, sequence, data, clientStartedAtUnixMs }` / `Ack { attachment_id, sequence }` / `Resize { owner, rows, cols }`。
- frame 上限は 64 KiB（`MAX_TERMINAL_REQUEST_BYTES`）。上限超過の text frame は分割されず、`INVALID_REQUEST` エラー応答になる。
- 認証は `Sec-WebSocket-Protocol: releash-bearer.<token>` で行い、handshake で同じ subprotocol を echo する。同時接続は 16 に制限される。

## token

- local API 起動時に master token と terminal 専用 token の 2 本を発行する。master token だけが discovery file に書かれる。
- terminal token は `TerminalStreamEndpoint`（`adaptor/controller/state.rs`）に保持され、Tauri command `get_terminal_stream_endpoint` が `{ url, authSubprotocol }` として renderer へ渡す。
- terminal route は master token と terminal token の両方を受理し、その他の route は master token だけを受理する。

## dispatch

- `adaptor/controller/command/mod.rs` の `CommandRouter` が、ドメインごとの `&'static [&'static str]`（command 名）と handler の表を持つ。
- handler の型は `Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync>` であり、`tauri::ipc::Invoke` に直接結合している。ws から同じ表を引いて同じ usecase を呼ぶ経路はない。
- 起動 gate（`gate_invoke_before_domain_routing`）も `tauri::ipc::Invoke` を前提にしている。

## push

- backend 状態の push は Tauri `AppHandle::emit` の直呼びと、ドメインごとの通知 trait 実装に分散している。単一の sink はない。
- backend 状態の push イベント名は次の 8 つである。
  - `agent-session-changed`（`adaptor/presenter/agent_session_changed.rs`）
  - `branch-list-sync`（`adaptor/gateway/repository/state.rs`）
  - `file-change`（`adaptor/gateway/repository/state.rs`、`adaptor/controller/command/watcher/mod.rs`）
  - `git-status-changed`（`adaptor/gateway/repository/state.rs`）
  - `repo-paths-changed`（`adaptor/gateway/repository/notify.rs`）
  - `repository-snapshot-changed`（`adaptor/gateway/repository/state.rs`）
  - `review-comments-changed`（`adaptor/controller/command/comment/commands.rs`、`infrastructure/comment/watcher.rs`）
  - `workflow-execution-changed`（`adaptor/gateway/workflow/state_notification_gateway.rs`）
- UI shell の事象である `menu-event`（`infrastructure/platform/menu.rs`）と `native-file-drop`（`infrastructure/platform/native_drop.rs`）も同じく `emit` 直呼びである。

## desktop の 1 スライス

- `get_current_branch` は `adaptor/controller/command/repository/` に登録され、`src/hooks/useCurrentBranch.ts` が Tauri `invoke` で呼ぶ。ws 経由の取得経路はない。現在ブランチを取得する command は他にない。
- `useCurrentBranch` は取得に失敗した場合、ブランチを `null` にしてブランチ名を表示しない。
- `workflow-execution-changed` は `src/hooks/useWorkflowState.ts` ほかが Tauri `listen` で購読する。ws 経由の購読経路はない。

## `[server]` 設定

- `adaptor/gateway/app_config/config_models.rs` に `ServerSection { bind, port, token, tls: TlsSection { enabled, cert, key } }` があり、`ReleashConfig` の `server` フィールドとして `releash.toml` に読み書きされる。既定値は `bind = "127.0.0.1"`、`port = 9700`。
- `domain/app_config/value_objects/mod.rs` に `ServerConfig` / `TlsConfig` があり、`AppConfigDocument` の `server` フィールドとして保持される。
- これらの値を読んで待ち受ける server は存在しない。`server.token` は空なら起動時に自動生成され、ログの秘匿対象リストへ入るだけである。`ServerConfig` は Tauri command の応答型に含まれず、frontend へ露出しない。

## レイテンシ

- ローカル ws の往復レイテンシの実測値は、リポジトリ内のどの文書にも記録されていない。

# Scope / Non-goals

## 変更するもの

- local API 上へのクライアント向け ws route の新設と、その bearer 認証。
- req/resp エンベロープ（`CommandRequest` / `CommandResponse`）と push 用エンベロープの定義。
- stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）と frame 上限・分割の規約の、エンベロープ定義への内包。
- `CommandRouter` の transport 非依存化。Tauri invoke と ws が同じ name→handler 表から同じ usecase を呼ぶ。
- push sink の単一化。backend 状態の push 8 イベントを 1 つの sink に寄せ、Tauri emit 実装と ws broadcast 実装を並置する。
- クライアント token の renderer への受け渡し。
- req/resp 1 本（`get_current_branch`）と push 1 本（`workflow-execution-changed`）の ws 経由化、および desktop の現在ブランチ表示の ws 経由化。
- `[server]` 設定（`ServerSection` の bind / port / token / tls と `domain::app_config` の `ServerConfig` / `TlsConfig`）の削除。
- ws 往復レイテンシ実測値の記録。

## 変更しないもの

- terminal 本体の汎用エンベロープへの移行。A2 で行う。
- stream フロー制御と frame 上限・分割の、送受信時の適用（上限超過時の分割、`ack` による流量制御）。A1 はエンベロープ定義としての規約までとし、適用は A2 で行う。
- frame 上限値の、renderer へ渡す接続情報としての配布。A1 はエンベロープ定義としての規約までとし、接続情報には含めない。
- ws 経由にする 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能。req/resp は Tauri invoke、push は Tauri emit / listen のまま維持する。既定経路の切替（A-flip）は後続で行う。
- ws 経由にした機能から Tauri invoke へ切り替えるフォールバック経路。設けない。
- renderer へ渡す token の追加発行。既存の非 master token（terminal 用）と別のクライアント token を新たに発行しない。
- 往復レイテンシの GO/NO-GO 判定基準（許容値）の設定。本変更では実測値の記録までとする。
- headless デーモンの抽出と常駐（LaunchAgent）。後続で行う。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口。
- `menu-event` / `native-file-drop`。UI shell の事象であり push sink 単一化の対象外とする。
- 既存設定ファイルの起動時書き換えによる `[server]` セクションの除去。読み込み時に除去のための書き換えは行わない。
- リモートアクセス経路。ローカル loopback 上の 1 クライアントだけを対象とする。

# Requirements

- R-001: local API 上にクライアント向けの WebSocket route があり、`Sec-WebSocket-Protocol` の bearer 方式で認証される。有効な token を提示した接続だけが確立する。
- R-002: クライアントは `CommandRequest{ request_id, command, args }` を送り、同じ `request_id` を持つ `CommandResponse{ request_id, result | error }` を受け取る。成功時は `result`、失敗時は `error` が返る。
- R-003: backend 状態の変化が、push 用エンベロープとして ws クライアントへ届く。push は typed を維持する。
- R-004: エンベロープ定義が、stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）と、frame 上限および上限超過時の分割の規約を含む。
- R-005: Tauri invoke から呼ばれた場合と ws から呼ばれた場合で、同じ command 名が同じ name→handler 表を引き、同じ usecase を呼ぶ。
- R-006: `agent-session-changed` / `branch-list-sync` / `file-change` / `git-status-changed` / `repo-paths-changed` / `repository-snapshot-changed` / `review-comments-changed` / `workflow-execution-changed` の 8 つの push が、単一の sink を経由して Tauri emit 実装と ws broadcast 実装の双方へ配られる。`menu-event` と `native-file-drop` はこの sink を経由しない。
- R-007: renderer は、master token ではないクライアント token を 1 本受け取る。その token でクライアント ws route と `/v1/terminal` の双方へ接続できる。master token は discovery file にだけ書かれ、renderer へは渡らない。
- R-008: desktop の現在ブランチ表示が、ws 経由の `get_current_branch` の req/resp で取得した値で行われる。
- R-009: desktop が `workflow-execution-changed` を ws 経由の push として受け取り、その状態変化が画面へ反映される。
- R-010: ws 経由にした 2 本以外の desktop 機能は、変更前と同じ経路・同じ結果で動作する。
- R-011: アプリケーションが扱う設定に `[server]` 設定（`ServerSection` の bind / port / token / tls、および `ServerConfig` / `TlsConfig`）が存在せず、アプリケーションが保存する設定ファイルに `[server]` セクションが含まれない。
- R-012: ws の往復レイテンシの実測値が `docs/specs/issues-1199/` 配下の文書に記録され、後続の予算判断から参照できる。
- R-013: ws 経由の現在ブランチ取得が失敗した場合、ブランチ名を表示しない（値なしとして扱う）。
- R-014: 旧 `[server]` セクションを含む既存の設定ファイルを読み込んでも失敗せず、`[server]` 以外の設定値は読み込み前の値のまま得られる。
- R-015: 既存の設定ファイルからの `[server]` セクションの除去は、設定を保存したときに行われる。保存されるまで既存の設定ファイルに `[server]` セクションが残る状態を受け入れる。

# Assumptions / Open Questions

すべて自動判断であり、人間の確認を経ていない。

- 自動判断（Q-001）: ws 経由にする req/resp 1 本は `get_current_branch` とする。Issue #1199 の完了条件が「desktop でブランチが ws 経由で表示される」と定めており、現在ブランチを取得する command は `get_current_branch` だけであるため、対象が一意に決まる。
- 自動判断（Q-002）: ws 経由にする push 1 本は `workflow-execution-changed` とする。Issue #1199 が挙げた唯一の候補であり、他を選ぶ根拠が正本にない。
- 自動判断（Q-003）: 往復レイテンシの実測値は `docs/specs/issues-1199/` 配下の文書に記録する。リポジトリ外（Issue コメント、milestone 説明文）への記録や、テストコード内の断定値としての固定は、対象範囲を広げるため採らない。
- 自動判断（Q-004）: 往復レイテンシの許容値（閾値）は定めない。正本に判定基準の記載がなく、閾値を置くと正本にない受入条件を増やすため、実測値の記録までを要求とする。
- 自動判断（Q-005）: ws 経由の現在ブランチ取得が失敗した場合はブランチ名を表示しない。これは変更前の `useCurrentBranch` が取得失敗時に取る挙動と同じであり、Tauri invoke へのフォールバック経路を設けると経路が二重になって対象範囲が広がるため採らない。
- 自動判断（Q-006）: stream フロー制御と frame 上限・分割は、A1 ではエンベロープ定義としての規約までとする。Issue #1199 は「terminal 本体の移行は A2」と定め、完了条件に送受信時の適用の検証を挙げていないため、適用は対象範囲に含めない。
- 自動判断（Q-007）: クライアント token は、既存の非 master token（terminal 用に発行している token）と同一の 1 本とする。判断⑦が求めるのは master token と分けることと master token を renderer へ露出させないことであり、既存の非 master token はいずれも満たす。3 本目を新たに発行すると renderer が持つ token が増え、判断⑧が定める最終状態（クライアント接続 1 本）から遠ざかるため採らない。
- 自動判断（Q-009）: frame 上限値は、renderer へ渡す接続情報に含めない。正本（Issue #1199、milestone 77 判断⑧）は frame 上限をエンベロープの規約として定めることを求めるだけで、接続情報として配布することを求めていない。R-007 が接続情報として定めるのはクライアント token であり、この上限値を利用する処理は A1 に存在しない。配布すると renderer 向けの wire contract に要求のない公開項目が増えるため採らない。
- 自動判断（Q-008）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。正本（Issue #1199）は削除対象として Rust の型（`ServerSection` の bind / port / token / tls、`ServerConfig` / `TlsConfig`）を挙げるだけで、既存ファイル上のデータの除去時期を定めていない。起動時の一括書き換えは、読み込みだけでは設定ファイルを書き換えないという変更前の挙動を変え、新しい観測可能な挙動を増やすため採らない。
