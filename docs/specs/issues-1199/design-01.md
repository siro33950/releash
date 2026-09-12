# Design 01

## 開始状態

- 初回。既存の `design-NN.md` はない。開始時点の実装は `docs/specs/issues-1199/requirements.md` の Current Behavior を参照する。
- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 未コミットの変更は `docs/specs/issues-1199/`（未追跡）だけで、コードの変更はない。この状態を今周の開始状態として扱う。
- 解消・見送りとなった Thread はない。今周は Review 由来ではなく、`[FIX_POLICY]` 付きの open Thread もない。

## 変える部分

- クライアント向け ws route の新設: local API 上にクライアント向け WebSocket route を追加し、bearer 認証を通す。根拠: R-001「local API 上にクライアント向けの WebSocket route があり、`Sec-WebSocket-Protocol` の bearer 方式で認証される」、B-001 / B-002。ルート: local API（`adaptor/controller/api/`、axum）の上に新設する。既存の `/v1/terminal` WebSocket（attach を `id` で相関し `status: attached | event | error` を返す）を雛形にする。認証は既存 terminal と同じ `Sec-WebSocket-Protocol` bearer 方式にする。旧 ws shell は #1338 で削除済みであり再利用しない。
- req/resp エンベロープの定義: `request_id` で要求と応答を相関するエンベロープを定義する。根拠: R-002「クライアントは `CommandRequest{ request_id, command, args }` を送り、同じ `request_id` を持つ `CommandResponse{ request_id, result | error }` を受け取る」、B-003 / B-004。ルート: `CommandRequest{ request_id, command, args }` / `CommandResponse{ request_id, result | error }` とする（判断①）。具体スキーマは委任。
- push 用エンベロープの定義: backend 状態の変化を ws クライアントへ届けるエンベロープを定義する。根拠: R-003「backend 状態の変化が、push 用エンベロープとして ws クライアントへ届く。push は typed を維持する」、B-005。ルート: push は typed を維持する（判断①）。具体スキーマは委任。
- stream 規約のエンベロープ内包: stream 単位のフロー制御と frame 上限・分割の規約をエンベロープ定義に含める。根拠: R-004「エンベロープ定義が、stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）と、frame 上限および上限超過時の分割の規約を含む」、B-006。ルート: `attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約をエンベロープに含める（判断⑧）。
- `CommandRouter` の transport 非依存化: name→handler 表を transport から切り離し、Tauri invoke と ws の双方から同じ表を引けるようにする。根拠: R-005「Tauri invoke から呼ばれた場合と ws から呼ばれた場合で、同じ command 名が同じ name→handler 表を引き、同じ usecase を呼ぶ」、B-007。ルート: `command/mod.rs` の `CommandRouter`（ドメイン別 name→handler 表）を `tauri::ipc::Invoke` から切り離し、Tauri invoke と ws が同じ表で同じ usecase を呼ぶ形にする。
- push sink の単一化: 分散している通知 trait と `app.emit` 直呼びを 1 つの sink に寄せる。根拠: R-006「8 つの push が、単一の sink を経由して Tauri emit 実装と ws broadcast 実装の双方へ配られる。`menu-event` と `native-file-drop` はこの sink を経由しない」、B-008 / B-009。ルート: 既存の通知 trait と `app.emit` 直呼びを 1 つの sink に寄せ、Tauri emit 実装と ws broadcast 実装を並置する。対象は backend 状態の push 8 イベント名で、`menu-event` / `native-file-drop` は対象外とする。sink の抽象は委任。
- クライアント token の renderer への受け渡し: renderer が master token ではない token 1 本でクライアント ws route と `/v1/terminal` の双方へ接続できるようにする。根拠: R-007「renderer は、master token ではないクライアント token を 1 本受け取る。その token でクライアント ws route と `/v1/terminal` の双方へ接続できる。master token は discovery file にだけ書かれ、renderer へは渡らない」、B-010 / B-011。ルート: クライアント token は local API 起動時に master token とは別に発行し、`TerminalStreamEndpoint` と同じ経路で renderer へ渡す（判断⑦）。master token を renderer へ露出させない。
- desktop の現在ブランチ表示の ws 経由化: `get_current_branch` の取得経路を ws の req/resp に置き換え、取得失敗時はブランチ名を表示しない。根拠: R-008「desktop の現在ブランチ表示が、ws 経由の `get_current_branch` の req/resp で取得した値で行われる」、R-013「ws 経由の現在ブランチ取得が失敗した場合、ブランチ名を表示しない」、B-012 / B-017。ルート: 委任
- desktop の `workflow-execution-changed` 購読の ws 経由化: 当該 push を ws 経由で受け取り、画面へ反映する。根拠: R-009「desktop が `workflow-execution-changed` を ws 経由の push として受け取り、その状態変化が画面へ反映される」、B-013。ルート: 委任
- `[server]` 設定の削除: 実体のない設定項目を取り除く。根拠: R-011「`[server]` 設定（`ServerSection` の bind / port / token / tls、および `ServerConfig` / `TlsConfig`）が存在しない」、B-015。ルート: 削除対象は `ServerSection` の bind / port / token / tls と `domain::app_config` の `ServerConfig` / `TlsConfig` とする。
- ws 往復レイテンシ実測値の記録: 実測値をリポジトリ内の文書へ残す。根拠: R-012「ws の往復レイテンシの実測値が `docs/specs/issues-1199/` 配下の文書に記録され、後続の予算判断から参照できる」、B-016。ルート: 記録先は `docs/specs/issues-1199/` 配下の文書とする。計測方法と記録文書のファイル名は委任。

## 固定するルート

人間が指定したルートは次のとおり。

- クライアント向け ws route は local API（`adaptor/controller/api/`、axum）の上に新設する。
- 既存の `/v1/terminal` WebSocket（attach を `id` で相関し `status: attached | event | error` を返す）をエンベロープの雛形にする。
- 認証は既存 terminal と同じ `Sec-WebSocket-Protocol` bearer 方式にする。
- エンベロープは `CommandRequest{ request_id, command, args }` / `CommandResponse{ request_id, result | error }` と push 用エンベロープを定義する（判断①）。push は typed を維持する。
- stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）と frame 上限・分割の規約をエンベロープに含める（判断⑧）。
- transport 非依存 dispatch は、`command/mod.rs` の `CommandRouter`（ドメイン別 name→handler 表）を `tauri::ipc::Invoke` から切り離し、Tauri invoke と ws が同じ表で同じ usecase を呼ぶ形にする。
- push sink の単一化は、既存の通知 trait と `app.emit` 直呼びを 1 つの sink に寄せ、Tauri emit 実装と ws broadcast 実装を並置する。対象は backend 状態の push 8 イベント名で、`menu-event` / `native-file-drop` は対象外とする。
- クライアント token は local API 起動時に master token とは別に発行し、`TerminalStreamEndpoint` と同じ経路で renderer へ渡す（判断⑦）。master token を renderer へ露出させない。
- 削除対象の dead な `[server]` 設定は、`ServerSection` の bind / port / token / tls と `domain::app_config` の `ServerConfig` / `TlsConfig` とする。
- 旧 ws shell は #1338 で削除済みであり再利用しない。

上記以外（エンベロープの具体スキーマ、型・モジュール配置、sink の抽象、ws クライアント側の実装、protocol テストの構成、レイテンシの計測方法と記録文書のファイル名）は委任であり、ルートを固定しない。

## 変えないもの

- ws 経由にする 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果を変えない。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しており、既定経路の切替（A-flip）は後続で行うため。
- `menu-event` / `native-file-drop` の既存経路を変えない。理由: UI shell の事象であり、push sink 単一化の対象外と決まっているため（R-006 / B-009）。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-001）: ws 経由にする req/resp 1 本を `get_current_branch` に確定した。現在ブランチを取得する command が他にないことを根拠にしており、この前提が外れると R-008 / B-012 の対象が変わる。
- 自動判断（Q-002）: ws 経由にする push 1 本を `workflow-execution-changed` に確定した。正本が挙げた唯一の候補という根拠であり、他の push を求められた場合は R-009 / B-013 の対象が変わる。
- 自動判断（Q-003）: 往復レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下の文書に確定した。別の記録先を求められた場合は R-012 / B-016 の充足先が変わる。
- 自動判断（Q-004）: 往復レイテンシの GO/NO-GO 許容値（閾値）は定めていない。閾値による判定が必要と判断された場合、A1 の受入条件だけでは GO/NO-GO を決められない。
- 自動判断（Q-005）: ws 経由の現在ブランチ取得が失敗した場合はブランチ名を表示しない（Tauri invoke へのフォールバックを設けない）。フォールバックが必要と判断された場合、R-013 / B-017 と Non-goals が変わる。
- 自動判断（Q-006）: stream フロー制御と frame 上限・分割は、A1 ではエンベロープ定義としての規約までとし、送受信時の適用は行わない。A1 で適用まで求められた場合、R-004 / B-006 の範囲が広がる。
- 自動判断（Q-007）: クライアント token は既存の非 master token（terminal 用に発行している 1 本）と同一とし、3 本目を発行しない。クライアント用に独立した token が必要と判断された場合、R-007 / B-011 の充足方法が変わる。
