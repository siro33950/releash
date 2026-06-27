# Behavior

要求 (`requirements.md`) を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

本 Issue は性能・メモリ効率改善であり、tool output 表示の **UI 仕様・見た目の変更は含まない**（requirements 非スコープ）。
振る舞い定義の中心は、巨大 tool output の保存境界を定義し、message part を full 本文ではなく
`content_ref` / `summary` / truncated `preview` 化した結果として観測できる以下の特性である:

- tool output が閾値を超えると full output が別 store へ退避され、part は ref + preview 化される。閾値未満は従来どおり inline 本文のまま（R1）。
- `MessagePart::ToolResult`（および tool 由来 part）が full 本文の代わりに `content_ref` / truncated `preview` / privacy-safe `summary` を保持でき、preview / summary に full 全長が含まれない（R2）。
- full output を message body とは別に保存・取得する `ToolOutputStore` 経路（id/ref ベースの遅延取得）が存在し、frontend は invoke 経由で取得する（R3 / Rust-first）。
- full output の保存先・retention・part に残す metadata が定義され、part に full 本文が残らず、session 削除時に当該 session の full output が整理される（R4）。
- message page API が巨大 tool output について preview + ref を返し、full output は明示要求時のみ読まれる（R5）。
- streaming delta（#1214）と WS 配信が巨大 tool output を全量 payload に載せず ref + preview で運び、reconnect / 通常配信のいずれでも full 全長が payload に比例しない（R6）。
- telemetry（#1209）で truncated count / full output bytes を観測でき、tool output 本文（ユーザーデータ）が通常ログ・span attribute・metric ラベルへ出ない（R7）。
- 上記の非退行・privacy を確認する検証手段が用意される（R8）。

## 仮定

- A1【確定】: 本 Issue の対象は agent message part（`MessagePart::ToolResult` 等）として保存・配信される tool result 本文（test / lint / shell / tool 由来）に限定する。terminal の live PTY output buffer 自体は #1215（実装済み）が担当し、本 behavior では扱わない。
- A2【確定】: full output の保存先は attachment store と同様の content-addressed file blob 参照方式（per-session ディレクトリ配下のファイル + ref id で遅延取得）とする。
- A3【確定】: truncate 閾値（max lines / max bytes）は「定義されていること」を振る舞いとして要求し、具体値は #1209 の performance budget 確定後に design / 実装で固定する。検討起点は OpenCode 相当（例: max ~1000 lines / ~30KB）。
- A4【確定】: retention は session ライフサイクル連動（session 削除時に当該 session の full output blob を削除）とし、古い tool output の積極的 pruning / compaction は本 Issue では非スコープ。
- A5: 閾値未満の小さい tool output は ref 化せず従来どおり part に inline 本文で保持する。
- A6: 「外部から観測可能な振る舞い」とは、message part に最終的に保持される表現（ref / preview / summary か inline 本文か）、page / streaming / WS payload に full 全長が載らないこと、full output が明示要求時のみ取得されること、通常ログ・telemetry にユーザーデータが出ないことを指す。内部の保存形式・呼び出し回数・clone 回数・具体的な閾値そのものは含めない。
- A7: WS 経路はモバイル向けフロント remote クライアントが削除済みのため、protocol / サーバー側が ref + preview で運べる状態にすることまでを範囲とし、受信クライアントを介した E2E 検証は行わず Rust 側の単体・結合テストで担保する。

Feature: 巨大 tool output の保存境界定義と message part の content-ref 化

  Background:
    Given Agent セッションで tool 由来の output（test / lint / shell / tool result）を保存・配信する経路が利用可能である
    And アプリは本 Issue（#1249）の tool output content-ref 化を適用済みのビルドで動作している
    And message page API・streaming delta 配信・WS 配信・full output 取得経路が利用可能である

  Rule: tool output は閾値で truncate 判定され、閾値未満は inline・閾値超過は別 store へ退避して ref + preview 化される

    Scenario: truncate 判定基準が定義されている
      Given tool output を保存する経路が動作している
      When tool output の保存境界（max lines / max bytes 等の閾値）を確認する
      Then truncate 判定に用いる閾値が定義されている

    Scenario: 閾値未満の小さい output は従来どおり inline 本文で保持される
      Given tool output のサイズ・行数がいずれも閾値未満である
      When その tool output を message part として保存する
      Then part は full output を inline 本文として保持する
      And full output の別 store への退避は行われない

    Scenario: 閾値超過の output は full output を別 store へ退避し part を ref + preview 化する
      Given tool output のサイズまたは行数が閾値を超過している
      When その tool output を message part として保存する
      Then full output は message body とは別の store へ退避される
      And part は full output 本文の代わりに ref + truncated preview を保持する

  Rule: ToolResult part は full 本文の代わりに content_ref・truncated preview・privacy-safe summary を保持できる

    Scenario: 閾値超過 part が ref・preview・summary を保持する
      Given 閾値を超過した tool output を退避済みである
      When 当該 part の保持内容を確認する
      Then part は full output の content_ref を持つ
      And part は先頭一定量の truncated preview を持つ
      And part は行数 / バイト数 / error フラグ等の privacy-safe summary を持つ

    Scenario: preview と summary に full output 全長が含まれない
      Given 閾値を大きく超過した tool output を退避済みである
      When part の preview と summary のサイズを確認する
      Then preview は先頭一定量に限定され、full output 全長を含まない
      And summary は集計値（行数 / バイト数 / error フラグ等）に限定され、full output 全長を含まない

  Rule: full output は message body と別に保存・取得され、frontend は invoke 経由で遅延取得する

    Scenario: full output を ref により別経路で遅延取得できる
      Given 閾値超過 part が full output の content_ref を保持している
      When その ref を指定して full output を取得する
      Then 退避された full output 全長が一意に取得される
      And この取得は message body（page / delta / snapshot）の取得経路とは独立している

    Scenario: full output 取得・保存ロジックが Rust 側に置かれ frontend は invoke で呼ぶ
      Given full output の保存・取得経路を実装している
      When 保存・truncate 判定・ref 解決・full output 取得のロジックの所在を確認する
      Then これらのロジックは Rust（usecase / domain）側に置かれている
      And frontend は full output を invoke 経由で取得し、表示用フォーマットのみを持つ

  Rule: full output の保存先・retention・残す metadata が定義され、part に full 本文が残らない

    Scenario: 退避後の part に full 本文が残らない
      Given 閾値超過 tool output を退避済みである
      When 当該 part を保存・配信の正典として観測する
      Then part には full output 本文が残らず、ref と privacy-safe metadata（preview / summary）のみが残る

    Scenario: session 削除時に当該 session の full output が整理される
      Given ある session に閾値超過 tool output の full output blob が退避されている
      When その session を削除する
      Then 当該 session に紐づく full output blob が併せて削除される

  Rule: message page API は巨大 tool output について preview + ref を返し full output は明示要求時のみ読む

    Scenario: page が巨大 tool output を preview + ref で返す
      Given 閾値超過 tool output を含む session の message page を取得する
      When page を取得する
      Then 巨大 tool output の part は preview と ref で返り、full output 全長は page payload に載らない
      And page payload は full output 全長に比例して増えない

    Scenario: 閾値未満の小さい output は page で従来どおり inline 本文で返る
      Given 閾値未満の tool output を含む session の message page を取得する
      When page を取得する
      Then その part は従来どおり inline 本文で返る

    Scenario: full output は frontend の明示要求時のみ読まれる
      Given page が preview + ref を返した状態である
      When frontend が ref を指定して full output を明示的に要求する
      Then その時に限り full output が読まれる
      And page 取得自体は full output を読まない

  Rule: streaming delta と WS 配信は巨大 tool output を ref + preview で運び full 全長を payload に載せない

    Scenario: 通常配信の delta に巨大 tool output 全長が載らない
      Given ストリーミング中に閾値超過の tool output が生じる
      When その tool output が delta として配信される
      Then 配信される delta は ref + preview を運び、full output 全長を payload に含まない
      And delta payload は full output 全長に比例して増えない

    Scenario: reconnect / resync 配信でも full output 全長が payload に載らない
      Given 閾値超過 tool output を含む session で reconnect / resync が発生する
      When resync 配信が行われる
      Then 配信 payload は ref + preview を運び、full output 全長を含まない
      And full output は delta / snapshot 経路ではなく full output 取得経路で読まれる

    Scenario: WS protocol / サーバー側が ref + preview で巨大 tool output を運べる
      Given WS 配信経路が利用可能である（受信クライアントは不在）
      When 閾値超過 tool output を WS protocol で表現する
      Then protocol / サーバー側は full output 全長を再送せず ref + preview で運べる
      And この経路は Rust 側 protocol / 配信ロジックの単体・結合テストで検証される

  Rule: telemetry で truncated count と full output bytes を観測でき、ユーザーデータが通常ログへ出ない

    Scenario: truncated count と full output bytes が観測できる
      Given #1209 telemetry が計装されている
      When 閾値超過 tool output を退避する
      Then truncated count（truncate 件数）が観測できる
      And full output bytes（store へ退避した総バイト数）が観測できる

    Scenario: tool output 本文が通常ログ・span attribute・metric ラベルへ出ない
      Given 閾値超過 tool output を退避・配信する
      When 通常ログ・span attribute・metric ラベルを確認する
      Then tool output 本文（ユーザーデータ）はいずれにも出力されない
      And telemetry は件数・バイト数等の集計値のみを持つ

  Rule: 非退行と privacy を確認する検証手段が用意される

    Scenario: session JSON / page / streaming payload が full 全長に比例しないことを検証できる
      Given 閾値超過 tool output を含むシナリオの検証手段が用意されている
      When 検証を実行する
      Then session JSON・page payload・streaming frame payload が full output 全長に比例しないことが確認される
      And full output が必要時のみ読まれることが確認される
      And ログにユーザーデータが出ないことが確認される

    Scenario: 既存テスト・lint が green であり新規ロジックにテストが追加される
      Given 本 Issue の変更を適用済みである
      When 既存のテスト（cargo test / pnpm test）と lint（cargo clippy -D warnings / pnpm lint）を実行する
      Then すべて成功する
      And truncate 判定・ref 退避・preview/summary 生成・full output 取得・session 削除連動の正常系・境界・異常系テストが追加されている

## Open Questions

なし（requirements の A1〜A6 で確定済み）。
