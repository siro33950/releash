# Behavior

要求（`requirements.md`）を、実装詳細を含まない観測可能な振る舞いとして定義する。対象は milestone 84 OB-2（stalled turn 中の送信でユーザー入力が失われる不具合）の解消。

## Feature: stalled turn 中の送信でもユーザー入力が失われない

Agent チャットで agent が長時間無反応（stall 判定中）のときにユーザーがメッセージを送信しても、送信操作は「turn 開始」または「queue 追加」のいずれかに収束し、入力テキスト・添付画像・mention が失われない。backend の内部エラー文言 `active-turn steering is not available` はユーザーに露出しない。

### Background

```gherkin
Background:
  Given ユーザーが Agent チャットセッションを開いている
```

## Rule: 実行中 turn への送信は成功以外の結果を持たない（I6）

steering 非対応の backend に対する実行中 turn への送信は、エラーにならず pending queue へ積まれる。stall 判定中の送信も同一の queue 経路に載る。

```gherkin
Scenario: 実行中 turn への送信が queue に積まれる
  Given agent が turn を実行中である
  And backend は実行中 turn への steering に対応していない
  When ユーザーが本文・添付画像・mention を含むメッセージを送信する
  Then メッセージはエラーにならず pending queue に積まれる
  And "active-turn steering is not available" エラーは返らない

Scenario: stall 判定中の送信も queue に積まれる
  Given agent が実行中 turn のまま stall 判定されている
  And backend は実行中 turn への steering に対応していない
  When ユーザーがメッセージを送信する
  Then メッセージはエラーにならず pending queue に積まれる
  And 送信操作は "turn 開始" または "queue 追加" のいずれかに収束する

Scenario: queue に積んだメッセージが turn 終了後に実行される
  Given stall 判定中の送信が pending queue に積まれている
  When 実行中 turn が終了する
  Then queue のメッセージが後続 turn として実行される
```

## Rule: queue に積むメッセージは欠落なく永続化される（R2）

queue へ積む際、human message は永続化され queue entry と紐づく。実行中送信と同じ扱いとし、本文・添付画像・mention・editor_context を保全する。

```gherkin
Scenario: queue 投入時にメッセージ内容が保全される
  Given ユーザーが本文・添付画像・mention・editor_context を含むメッセージを送信する
  And その turn は stall 判定中である
  When メッセージが pending queue に積まれる
  Then 本文・添付画像・mention・editor_context が欠落なく保持される
  And 永続化された human message が queue entry と紐づく
```

## Rule: 入力欄は送信成功時にのみクリアされる（P5 / R3・R4）

送信ハンドラは送信の完了を待ち、成功応答を得たときにのみ入力欄・添付画像・pasted text・mention をクリアする。送信が失敗した場合はこれらを保持する。

```gherkin
Scenario: 送信成功後に入力欄がクリアされる
  When ユーザーがメッセージを送信し、送信が成功応答を返す
  Then 入力欄・添付画像・pasted text・mention がクリアされる

Scenario: 送信失敗時に入力欄が保持される
  When ユーザーがメッセージを送信し、送信が失敗する
  Then 入力欄・添付画像・pasted text・mention は保持される
  And ユーザーは失われていない入力から再送できる

Scenario: 送信中に入力が即時破棄されない
  When ユーザーが送信操作を行う
  Then 送信の結果が確定するまで入力欄・添付は破棄されない

Scenario: 送信待機中の追加入力が先行送信の成功で失われない
  Given メッセージの送信結果が未確定である
  When ユーザーが入力・添付を追加して再度送信操作を行う
  Then 未確定のメッセージは重複送信されない
  And 追加入力・添付は先行送信の成功後も保持される
```

## Rule: queue に積まれた送信はユーザーに見える（R5）

stall 中の送信が queue チップとして UI に表示され、メッセージが失われていないことがユーザーから分かる。

```gherkin
Scenario: stall 中の送信が queue チップとして表示される
  Given agent が stall 判定中である
  And backend は実行中 turn への steering に対応していない
  When ユーザーがメッセージを送信する
  Then そのメッセージが queue チップとして UI に表示される
  And ユーザーはメッセージが保持されていることを確認できる
```

## Rule: backend の生エラー文言はユーザーに露出しない（R6）

`active-turn steering is not available` を含む backend 内部エラー文言は、ユーザー向け UI にそのまま表示されない。

```gherkin
Scenario: steering 非対応の生エラーがユーザーに露出しない
  Given agent が実行中 turn または stall 判定中である
  And backend は実行中 turn への steering に対応していない
  When ユーザーがメッセージを送信する
  Then "active-turn steering is not available" 文言はユーザー向け UI に表示されない
```

## 主要な境界条件

```gherkin
Scenario: stall 判定中の実 steer 対応 backend では steer 経路が優先される
  Given agent が実行中 turn のまま stall 判定されている
  And backend が実行中 turn への steering に対応している
  When ユーザーが stalled turn へメッセージを送信する
  Then 送信は steer 経路で処理され、queue フォールバックは適用されない

Scenario: turn 非実行時の送信は通常どおり turn を開始する
  Given agent が turn を実行していない
  When ユーザーがメッセージを送信する
  Then 送信は新しい turn を開始する
```

## 仮定

- 「queue へ積む」対象は、steering 非対応 backend の実行中 turn（stall 判定中を含む）への送信とする。将来 backend が実 steer を実装した場合も、本 ISSUE で steer 優先を保証するのは stall 観測中の active turn に限る。通常の streaming turn は既存どおり queue へ積む。
- queue 投入時の human message 永続化・queue 紐付けは、既存の実行中送信と同一の仕組みを再利用する（新規の永続化スキーマは追加しない）。
- pending queue の永続化（session close / backend 切替 / 再起動での queue 消滅、OB-3）は本 ISSUE の対象外であり、振る舞いとして定義しない。
- issues-1301 D16/F-2 の「stalled retry/continue must not be silently queued」仕様は本 ISSUE で反転され、stall 中の送信も queue へフォールバックする。当該テストは新仕様に合わせて更新／置換する。

## Open Questions

なし。
