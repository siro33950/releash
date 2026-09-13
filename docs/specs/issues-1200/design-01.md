# Design 01

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 初回。直前の Design はない。開始状態は `requirements.md` の Current Behavior（`28f0eeed` 時点）のとおりで、Spec 工程ではコードを変更していない。作業ツリーの未コミットの変更は `docs/specs/issues-1200/` 配下の文書だけである。
- open Thread はない。解消・見送りとなった Thread もない。
- Requirements R-001〜R-012 と Behavior B-001〜B-014 は対応表で欠落なく対応しており、修正を要する誤り・不足・矛盾はない。

## 変える部分

- エンベロープの Protocol Buffers 化: クライアント ws の req/resp・push・stream の frame を、`.proto` に定義したメッセージのバイナリ frame にする。根拠: R-003「クライアント ws の req/resp・push・stream の frame は、`.proto` に定義された Protocol Buffers メッセージのバイナリである」、B-003。ルート: 「固定するルート」の判断①
- 対象ドメインの command のクライアント ws 被覆: repository / code / comment / agent_session / terminal_surface / workflow / workspace_tree / workspace_state / app_config / notion / git_host / external_editor / telemetry / watcher / application_lifecycle に登録された command を、クライアント ws の汎用エンベロープで `request_id` 付きの要求として呼べ、同じ `request_id` の応答で成功値またはエラーが返り、その結果が Tauri invoke から呼んだ場合と一致するようにする。desktop から呼び出す箇所がない 34 本（Current Behavior に列挙）はこの項目に含めない（「未確定・リスク」の Q-004）。根拠: R-001、R-002、B-001、B-002。ルート: 「固定するルート」の判断①・対象ドメインの単位・検証方法
- terminal のクライアント ws 汎用エンベロープへの移行: terminal の attach・出力 stream・入力・ack・resize を、req/resp と push と同じ 1 本のクライアント接続上で行えるようにする。根拠: R-004「terminal の attach・出力 stream・入力・ack・resize が、クライアント ws の汎用エンベロープで、req/resp と push と同じ 1 本のクライアント接続上で行える」、B-004。ルート: 「固定するルート」の判断⑧・terminal_surface
- stream フロー制御の送受信時の適用: terminal の stream を `attachment_id` ごとに `sequence` で順序付けて送り、受信側が `attachment_id` と受信済みの `sequence` を `ack` として通知できるようにする。根拠: R-005、B-005。ルート: 「固定するルート」の判断⑧
- frame 上限・分割の送受信時の適用: frame 上限を超えるデータを規約に従って分割して送り、各 frame を上限以下にし、受信側で欠落と順序の入れ替わりなく元のデータを得られるようにする。根拠: R-006、B-006。ルート: 委任
- `/v1/terminal` route と `get_terminal_stream_endpoint` の削除: local API から `/v1/terminal` route を除き、`get_terminal_stream_endpoint` を Tauri invoke で呼ぶと接続情報でなくエラーが返るようにする。根拠: R-007「local API に `/v1/terminal` route が存在せず、`get_terminal_stream_endpoint` も呼べない」、B-007、B-010。ルート: 「固定するルート」の terminal_surface
- desktop terminal の Tauri Channel fallback の削除: `src/hooks/useTerminal.ts` の terminal が、Tauri Channel を使わずクライアント ws だけで出力の受信と入力の送信を行うようにする。根拠: R-008、B-008。ルート: 「固定するルート」の terminal_surface
- 切断・接続失敗後の terminal の再 attach: desktop の terminal が、クライアント ws の予期しない切断の後、またはクライアント ws に接続できず attach できなかった後に、クライアント ws の接続が確立した時点でクライアント ws 上で（再）attach し、出力を再同期して表示と入力を再開するようにする。根拠: R-012、B-013、B-014。ルート: 委任
- desktop の UI 機能の呼び出しのクライアント ws 経由への差し替え: desktop の UI 機能が対象ドメインの command を呼ぶ操作を、Tauri invoke からクライアント ws の要求へ差し替え、変更前と同じ結果を画面に反映する。`get_application_startup_outcome` と `quit_after_startup_failure` は差し替えない（R-011）。desktop から呼び出す箇所がない 34 本は呼び出し箇所が無いためこの項目の対象にならない。根拠: R-010、B-011。ルート: 委任

## 固定するルート

- 判断①（milestone 77）: req/resp は Protocol Buffers で定義したエンベロープ（`request_id`＋`oneof command`）を経由して usecase 共有 dispatch へ渡す。push も proto message とする。`.proto` を protocol の正とし、Rust / client の型はそこから生成する。
- 判断⑧（milestone 77）: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。
- 判断⑦（milestone 77）: クライアント token は discovery file の master token と分けて発行し、master token を renderer に露出させない。
- 土台（milestone 77）: クライアント向け ws は MS82 で新設した local API（axum、127.0.0.1 bind、discovery file）の上のものを使う。A0 で温存した旧 ws shell（#1338 で削除済み）は再利用しない。
- 対象ドメインの単位（Issue #1200）: 被覆対象のドメインは `adaptor/controller/command/` の登録単位とする。
- terminal_surface（Issue #1200）: `/v1/terminal` の独自 route をエンベロープへ移行し、旧 route と Tauri Channel fallback を削除する。
- 検証方法（Issue #1200 完了条件）: 各ドメインの protocol テスト（usecase 結果との parity）で検証する。

## 変えないもの

- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と結果。理由: milestone 77 の土台が CLI と provider hook のローカル専用入口として残すと定め、Issue #1200 の workflow の項が CLI 用の HTTP endpoint を残すと定めているため（R-009、B-009）。

## 未確定・リスク

`[DEFERRED]` で人間へ渡した件はない。この周までの自動判断はいずれも人間の確認を経ていない。

### 未決のまま残した要求

- Q-004（R-001、R-002、B-001、B-002 のうち該当 34 本の部分）: desktop から呼び出す箇所がない登録済み command 34 本（`requirements.md` の Current Behavior に列挙）を、(a) 他の command と同様にクライアント ws へ被覆する、(b) 被覆せず Tauri command の登録ごと削除する、(c) 被覆せず Tauri command として現状のまま残す、のどれにするか。今周の「変える部分」には含めていない。(a) と決まれば 34 本の被覆が次の周の変更になり、(b) と決まれば登録の削除と、R-001 / R-002 の対象範囲の見直しが必要になる。

### 自動判断

- Q-001: クライアント ws のエンベロープ（req/resp、push、stream）をこの変更で Protocol Buffers バイナリ frame に置き換える。A1（PR #1768）と `docs/specs/issues-1199/` は JSON text frame を前提としている。
- Q-002: desktop の該当 UI 機能の呼び出しをこの変更でクライアント ws 経由へ差し替える。Issue #1201（A-flip）は呼び出し側の差し替えを A-flip の作業に挙げている。
- Q-002 に伴う判断: `get_application_startup_outcome` と `quit_after_startup_failure` の desktop からの呼び出しは Tauri invoke のまま維持する。
- Q-003: telemetry / watcher / application_lifecycle は登録済みの全 command を被覆し、UI shell に残す分の線引きは A-flip で決める。
- Q-005: Tauri Channel fallback の削除後、クライアント ws の切断・接続失敗の後に接続が確立した時点で terminal を再 attach・再同期する。attach 要求にエラー応答が返った場合の挙動は定めていない。
- `get_terminal_stream_endpoint` を `/v1/terminal` route とともに削除する。
- terminal の Tauri Channel fallback の削除をこの変更で行う。Issue #1201 も同じ撤去を A-flip の作業に挙げている。
- 作業単位を Issue #1200 の対象ドメインすべてとする。

### 既存 E2E との整合

- Playwright の既存 E2E は `tests/helpers/tauri-mock.ts` で、Tauri IPC を mock し、`/v1/client` を `page.routeWebSocket` で JSON text frame として mock している。エンベロープの Protocol Buffers 化と desktop の呼び出しの ws 経由への差し替えにより、この mock の前提（JSON text frame、IPC mock で応答する command）が成り立たなくなる。Requirements の Q-002 はこの整合を Design で扱うとしているが、実装上のルートは人間が指定しておらず委任である。Tauri IPC mock に代わる E2E 経路の置き換えは Non-goals（A-flip）のままである。
