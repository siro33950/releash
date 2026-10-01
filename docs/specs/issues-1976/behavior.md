## B-001: 技術的な失敗の性質がステータスコードに出る

GIVEN 変更対象の箇所の下の層が、性質を持つ技術的な失敗を返す
WHEN 画面がその操作を呼び出す
THEN 画面に返るステータスコードは、その性質に対応するコード（Transient は UNAVAILABLE、TimedOut は DEADLINE_EXCEEDED、Cancelled は CANCELED、Other は INTERNAL）である
AND 失敗のメッセージには元の失敗のメッセージが残る

## B-002: 業務の失敗は業務の結果として届く

GIVEN 変更対象の箇所の下の層が、入力の誤り・対象が無い等の業務の失敗を返す
WHEN 画面がその操作を呼び出す
THEN 画面に返るステータスコードは、その業務の失敗に対応するコードであり、「使えない」系の固定のコードではない

## B-003: AgentSession の Provider と Terminal の操作の失敗の文言

GIVEN AgentSession の Provider の操作または Terminal の操作が技術的な失敗で終わる
WHEN 画面がその操作を呼び出す
THEN 画面に出る文言は、Provider の操作または Terminal の操作を完了できなかった旨の今の文言である
AND ステータスコードは元の失敗の性質に対応するコードである

## B-004: terminal の対象が無い失敗と外部の失敗の区別

GIVEN terminal の操作で、対象の Terminal Surface が無い
WHEN 画面がその操作を呼び出す
THEN 対象が無い業務の失敗に対応するステータスコードが返る

## B-005: terminal の外部の失敗の性質

GIVEN terminal の操作（起動の経路を含む）で、checkpoint のファイル操作または PTY の操作が外部の失敗で終わる
WHEN 画面がその操作を呼び出す
THEN 元の失敗の性質に対応するステータスコードが返る

## B-006: 起動時の store の失敗の分類

GIVEN 起動時に store を開く処理で、reader の接続が SQLite の版が古いために失敗する
WHEN daemon が起動する
THEN 起動の失敗の kind は UnsupportedRuntime である

## B-007: 起動の失敗の「次回の起動で直るか」

GIVEN 起動時に store を開く処理が StorageUnavailable で失敗する
WHEN daemon が起動の失敗を画面へ伝える
THEN 元の性質が Transient または TimedOut なら retry_on_next_launch は true である
AND それ以外の性質なら retry_on_next_launch は false である

## B-008: 起動の失敗のうち性質によらない kind

GIVEN 起動時に store を開く処理が StoreInUse または SchemaEvolutionFailed で失敗する
WHEN daemon が起動の失敗を画面へ伝える
THEN retry_on_next_launch は true である

## B-009: 起動時と実行中で同じ失敗の性質が同じ

GIVEN 同じ SQLite の失敗、または同じ io::Error が、起動時の store を開く処理と、実行中の store の書き込み・読み取りで起きる
WHEN それぞれが失敗の性質を決める
THEN 両者の性質は同じである（SQLite の busy は Transient、inaccessible は Other。io::Error の Interrupted・WouldBlock・接続の切断は Transient、TimedOut は TimedOut、それ以外は Other）

## B-010: やり直しの判断は元の性質で決まる

GIVEN 変更対象の箇所の下の層が技術的な失敗を返し、その失敗がやり直しの判断に渡る
WHEN やり直すかどうかを判断する
THEN 元の性質が Transient ならやり直す
AND それ以外の性質ならやり直さない

## B-011: 画面のつなぎ直しが変わらない

GIVEN この変更でステータスコードが変わる失敗が起きる
WHEN 画面がその失敗を受け取る
THEN 画面がつなぎ直す・つなぎ直さないは変更前と同じである

## B-012: CLI のエラーの対応

GIVEN CLI が HTTP local API から失敗の HTTP status を受け取る
WHEN CLI がそれを CLI のエラーにする
THEN 404 は NotFound、400・409・422 は InvalidInput、それ以外は Other になる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-001, B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005 |
| R-005 | B-006, B-007 |
| R-006 | B-007, B-008 |
| R-007 | B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-012 |
