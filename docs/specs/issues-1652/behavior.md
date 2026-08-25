## B-001: coded error の表示文言

GIVEN Tauri command が `code` と `message` を持つ coded error で reject する
WHEN 利用者へそのエラーが表示される
THEN 表示文言は backend が返した `message` と一致する
AND `[object Object]` は表示されない

## B-002: プレーン文字列 error の表示文言

GIVEN Tauri command がプレーン文字列で reject する
WHEN 利用者へそのエラーが表示される
THEN 表示文言は reject された文字列と一致する

## B-003: Terminal 上限到達時の初期化文言

GIVEN Terminal 初期化が `CAP_REACHED` の coded error で失敗する
WHEN 利用者が失敗文言を受け取る
THEN 文言は backend が返した `message` と一致する
AND 文言だけで Terminal の上限到達だと判別できる

## B-004: Terminal 上限到達以外の初期化失敗文言

GIVEN Terminal 初期化が `CAP_REACHED` 以外の backend error で失敗する
WHEN 利用者が失敗文言を受け取る
THEN 文言は backend が返した `message` と一致する
AND 文言だけで Terminal 初期化失敗だと判別できる

## B-005: 稼働中 Terminal の再同期失敗文言

GIVEN 稼働中 Terminal の attachment を再同期する必要がある
WHEN 再同期用の `attach_terminal_surface` が coded error で失敗する
THEN 文言は backend が返した `message` と一致する
AND 文言だけで初期化失敗ではなく再同期失敗だと判別できる

## B-006: coded error の利用者向け文言

GIVEN backend が coded error を返す
WHEN その `message` が UI に表示される
THEN 表示文言は既存 UI と同じ英語である
AND 利用者が UI 上の対象と発生した事象を識別できる
AND 回復可能な場合は利用者が取れる行動を識別できる
AND 実装内部の語彙だけで構成された文言ではない

## B-007: stale review group target からの回復

GIVEN stage または unstage の対象が stale である
WHEN Tauri command が `STALE_REVIEW_GROUP_TARGET` の coded error で reject する
THEN reject 値の `code` は機械可読な値として利用できる
AND frontend は snapshot を再取得する

## B-008: coded error の telemetry 報告

GIVEN frontend が backend 由来の coded error を telemetry として報告する
WHEN telemetry の報告内容が backend へ送られる
THEN 報告される `message` は coded error の `message` と一致する
AND `[object Object]` は報告されない

## B-009: backend 由来エラーを扱う全操作での一貫性

GIVEN 利用者が Releash desktop で backend の処理を伴う操作を行う
WHEN その操作が失敗し、利用者へエラーが表示されるか telemetry として報告される
THEN coded error では backend が返した `message` と一致する文言が使われる
AND プレーン文字列 error では reject された文字列と一致する文言が使われる
AND `[object Object]` は表示も報告もされない

## B-010: type tagged error の表示・報告文言

GIVEN application lifecycle command が `type` と `message` を持つ tagged error で reject する
WHEN そのエラーが利用者へ表示されるか telemetry として報告される
THEN 使用される文言は backend が返した `message` と一致する
AND `[object Object]` は表示も報告もされない
AND `type` と variant 固有の既存フィールドは機械可読な値として維持される

## B-011: Terminal WebSocket error の操作別文言

GIVEN Terminal の attach、write、resize、または不正 request が WebSocket transport で失敗する
WHEN 利用者が失敗文言を受け取る
THEN 文言は Rust がその操作に対して返した固定 `message` と一致する
AND frontend が接頭辞、接尾辞、または言い換えを付加しない
AND attach、write、resize 失敗の文言は Tauri command transport と一致する

## B-012: Terminal 失敗表示からの回復

GIVEN Terminal の失敗文言が表示されている
WHEN 同じ Terminal の後続の初期化・attach が完走するか、現行 attachment epoch の再同期が成功する
THEN その失敗文言は表示されなくなる
AND 古い attachment epoch の成功は現行の失敗文言を消さない

## B-013: Terminal 入力不能 stream item の利用者向け文言

GIVEN Terminal 入力が stale attachment、pending capacity 超過、または runtime write 失敗により受理できない
WHEN `input_unavailable` stream item が Tauri Channel または local API WebSocket で利用者へ届く
THEN `message` は Rust の protocol 境界が返した `Terminal input could not be sent. Try again.` と一致する
AND 利用者が Terminal 入力を送信できなかった事象と再試行できることを識別できる
AND attachment、reorder buffer、PTY writer などの内部原因は wire の `message` に含まれない
AND transport によって `message` が変わらない

## 要件 ID と Behavior ID の対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-009 |
| R-002 | B-002, B-009 |
| R-003 | B-001, B-002, B-003, B-004, B-005, B-009, B-010, B-011, B-013 |
| R-004 | B-003, B-004 |
| R-005 | B-005, B-006, B-010, B-011, B-013 |
| R-006 | B-007 |
| R-007 | B-008, B-009, B-010 |
| R-008 | B-009, B-010, B-011, B-013 |
| R-009 | —（内部可観測性のため behavior 対象外） |
| R-010 | B-009, B-010 |
| R-011 | B-005, B-011 |
| R-012 | B-012 |
| R-013 | B-013 |
