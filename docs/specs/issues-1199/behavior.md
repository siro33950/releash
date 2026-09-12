## B-001: クライアント ws 接続の認証成立

GIVEN local API が起動しており、クライアント向け ws route が存在する
WHEN クライアントが有効なクライアント token を `Sec-WebSocket-Protocol` の bearer として提示して接続する
THEN WebSocket 接続が確立する

## B-002: 無効な token による接続の拒否

GIVEN local API が起動しており、クライアント向け ws route が存在する
WHEN クライアントが token を提示せずに接続する、または無効な token を提示して接続する
THEN WebSocket 接続は確立しない

## B-003: req/resp の相関と成功応答

GIVEN クライアント ws 接続が確立している
WHEN クライアントが `CommandRequest{ request_id, command, args }` を送る
THEN 同じ `request_id` を持つ `CommandResponse` が返る
AND その `CommandResponse` は `result` を持つ

## B-004: req/resp の失敗応答

GIVEN クライアント ws 接続が確立している
WHEN クライアントが送った `CommandRequest` の処理が失敗する
THEN 同じ `request_id` を持つ `CommandResponse` が返る
AND その `CommandResponse` は `error` を持つ

## B-005: backend 状態変化の push 受信

GIVEN クライアント ws 接続が確立している
WHEN backend 状態の push 対象となる状態変化が起きる
THEN その変化が push 用エンベロープとしてクライアントへ届く
AND push の内容は typed なペイロードとして届く

## B-006: stream フロー制御と frame 規約

GIVEN クライアント ws のエンベロープ定義が存在する
WHEN stream を伴うやり取りをエンベロープで表す
THEN `attachment_id` と `sequence` で stream ごとの順序が識別できる
AND `ack` で受信済みの `sequence` を通知できる
AND frame 上限と、上限を超える場合の分割の扱いが規約として定まっている

## B-007: transport をまたいだ同一結果

GIVEN 同じ command 名が Tauri invoke と ws の双方から呼べる
WHEN 同じ引数でその command を Tauri invoke 経由と ws 経由でそれぞれ呼ぶ
THEN どちらも同じ usecase の結果を返す

## B-008: push sink の単一化

GIVEN Tauri emit 実装と ws broadcast 実装が単一の sink に並置されている
WHEN `agent-session-changed` / `branch-list-sync` / `file-change` / `git-status-changed` / `repo-paths-changed` / `repository-snapshot-changed` / `review-comments-changed` / `workflow-execution-changed` のいずれかを発生させる状態変化が起きる
THEN その push が Tauri emit の購読者へ届く
AND その push が ws broadcast の購読者へ届く

## B-009: UI shell 事象の対象外

GIVEN push sink が単一化されている
WHEN `menu-event` または `native-file-drop` が発生する
THEN それらは push sink を経由せず、従来どおり UI shell の経路で届く

## B-010: master token の非露出

GIVEN local API が起動している
WHEN discovery file を読む
THEN discovery file には master token がある
AND renderer は master token を取得できない

## B-011: renderer へのクライアント token の受け渡し

GIVEN local API が起動している
WHEN renderer が `TerminalStreamEndpoint` と同じ経路で接続情報を取得する
THEN master token ではないクライアント token を 1 本含む接続情報が得られる
AND その token でクライアント ws route へ接続できる
AND その token で `/v1/terminal` へ接続できる

## B-012: desktop のブランチ表示が ws 経由

GIVEN desktop が起動し、リポジトリが選択されている
WHEN 現在ブランチを表示する
THEN ブランチ名が ws 経由の `get_current_branch` の req/resp で取得された値として表示される

## B-013: desktop が push を ws 経由で受信

GIVEN desktop が起動し、クライアント ws 接続が確立している
WHEN `workflow-execution-changed` を発生させる状態変化が起きる
THEN その変化が ws push として desktop に届き、画面へ反映される

## B-014: ws 経由にした 2 本以外の維持

GIVEN desktop が起動している
WHEN `get_current_branch` と `workflow-execution-changed` 以外の機能を操作する
THEN 変更前と同じ経路で処理され、変更前と同じ結果になる

## B-015: `[server]` 設定の消滅

GIVEN アプリケーション設定を扱う
WHEN アプリケーションが設定を保存する
THEN 保存された設定ファイルに `[server]` セクションおよび bind / port / token / tls の設定項目は存在しない

## B-016: レイテンシ実測値の記録

GIVEN クライアント ws 経由の req/resp が動作している
WHEN 往復レイテンシを計測する
THEN その実測値が `docs/specs/issues-1199/` 配下の文書に記録されている

## B-017: ws 経由のブランチ取得失敗時の表示

GIVEN desktop が起動し、リポジトリが選択されている
WHEN ws 経由の `get_current_branch` の req/resp が失敗する
THEN ブランチ名は表示されない

## B-018: 旧 `[server]` を含む既存設定ファイルの読み込み

GIVEN 旧 `[server]` セクションを含む設定ファイルが存在する
WHEN アプリケーションがその設定ファイルを読み込む
THEN 読み込みは失敗しない
AND `[server]` 以外の設定値は読み込み前の値のまま得られる

## B-019: 読み込みだけでは旧 `[server]` を除去しない

GIVEN 旧 `[server]` セクションを含み、他の移行を要しない設定ファイルが存在する
WHEN アプリケーションが設定を読み込み、設定を保存しない
THEN 設定ファイルの `[server]` セクションは残る

## B-020: 保存による旧 `[server]` の除去

GIVEN 旧 `[server]` セクションを含む設定ファイルが存在する
WHEN アプリケーションが設定を保存する
THEN 設定ファイルから `[server]` セクションおよび bind / port / token / tls の設定項目が除去される
AND `[server]` 以外の設定値は保持される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
| R-006 | B-008, B-009 |
| R-007 | B-010, B-011 |
| R-008 | B-012 |
| R-009 | B-013 |
| R-010 | B-014 |
| R-011 | B-015 |
| R-012 | B-016 |
| R-013 | B-017 |
| R-014 | B-018 |
| R-015 | B-019, B-020 |
