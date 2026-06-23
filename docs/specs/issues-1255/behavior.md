# Behavior

requirements.md の要求を、観測可能な振る舞いとして Gherkin で定義する。実装上の実現方式（turn generation / interrupt seq / session resume 方針など）は本書では規定せず、`design.md` で決定する。

## 仮定

- 対象 backend は Claude backend（`claude-sdk-bridge`）に限定する（requirements の仮定に従う）。以下のシナリオはすべて Claude backend の AgentChat session を前提とする。
- 「Stop ボタンが送信ボタンに戻った後」とは、AgentChat の入力エリアが停止操作可能状態（Stop ボタン表示）から送信可能状態（送信ボタン表示）へ戻った状態を指す。
- 「Stop 前 turn の続きを返さない」ことの外形的判定は、次送信後に返る応答が「新しいユーザー入力に対応した内容」であり、かつ「Stop 前 turn の未完了出力の継続ではない」ことで確認する（requirements の仮定に従う）。
- 「late event（late stream / late result / late turn_complete）」とは、Stop によって中断した turn に由来し、Stop 操作より後に到着する出力・完了イベントを指す。
- 「completed と誤認されない」とは、Stop で中断した turn の状態が、通常の正常完了 turn と同一の完了状態として表示・集約・通知されないことを指す。

---

## Feature: Stop を turn のインタラプト境界として扱う

AgentChat（Claude backend）で応答中に Stop を行い、送信ボタンに戻った後に新しいメッセージを送ったとき、Stop 前 turn の続きではなく、新しいユーザー入力への応答が開始される。Stop された turn は通常完了 turn と区別され、Stop 以前の遅延イベントは次 turn に混入しない。

### Background

```gherkin
Background:
  Given Claude backend の AgentChat session が開いている
  And ユーザーがメッセージを送信して turn が応答中（streaming）である
```

---

### Rule: Stop 後の次送信は新しいユーザー入力への応答として開始される

```gherkin
Scenario: 送信ボタンに戻った後の次送信が Stop 前 turn の続きを返さない
  Given ユーザーが応答中に Stop を行った
  And 入力エリアが送信ボタン表示に戻っている
  When ユーザーが新しいメッセージを送信する
  Then 返る応答は新しいメッセージに対応した内容である
  And Stop 前 turn の未完了出力は継続されない

Scenario: 次送信で停止済み turn の成功完了 resume に起因する長時間待機が発生しない
  Given ユーザーが応答中に Stop を行った
  And 入力エリアが送信ボタン表示に戻っている
  When ユーザーが新しいメッセージを送信する
  Then 停止済み turn を成功完了として resume したことに起因する長時間待機は発生しない
  And 応答は新しいメッセージへの応答として開始される
```

---

### Rule: Stop された turn は通常完了 turn と区別される（interrupted 境界）

```gherkin
Scenario: Stop された turn が正常完了 turn として扱われない
  Given ユーザーが応答中に Stop を行った
  Then Stop された turn は interrupted（中断）として扱われる
  And Stop された turn は正常完了（成功終了）turn として扱われない

Scenario Outline: interrupted turn が completed と区別される観測面
  Given ユーザーが応答中に Stop を行った
  Then <観測面> において interrupted turn は通常の completed turn と区別される

  Examples:
    | 観測面               |
    | pending queue        |
    | workflow 通知        |
    | status 遷移          |
```

---

### Rule: Stop 以前に発生した late event は次 turn に混入しない

```gherkin
Scenario: Stop 前の late stream / late result が次 turn の agent message に混入しない
  Given ユーザーが応答中に Stop を行った
  And Stop で中断した turn に由来する late stream / late result が Stop 後に到着する
  When ユーザーが新しいメッセージを送信する
  Then late stream / late result は新しい turn の agent message に混入しない

Scenario: Stop 後に到着した late turn_complete が新しい turn を完了扱いにしない
  Given ユーザーが応答中に Stop を行った
  When ユーザーが新しいメッセージを送信して新しい turn が応答中になる
  And Stop で中断した turn に由来する遅延 turn_complete が到着する
  Then 遅延 turn_complete は新しい turn を完了扱いにしない
  And 新しい turn は自身の応答に基づいて完了する

Scenario: Stop 後の次送信が旧 turn の pending / stream に混入しない
  Given ユーザーが応答中に Stop を行った
  When ユーザーが新しいメッセージを送信する
  Then 新しい送信は旧 turn の pending / stream に混入しない
```

---

### Rule: Stop 後の状態は completed と誤認されない

```gherkin
Scenario Outline: Stop 後の状態が completed と誤認されない
  Given ユーザーが応答中に Stop を行った
  Then <表示先> において Stop 後の状態は completed と誤認されない

  Examples:
    | 表示先              |
    | UI                  |
    | SessionStore        |
    | AgentStatusCenter   |
```

---

### Rule: 既存挙動を維持する

```gherkin
Scenario: 通常 turn 完了挙動を壊さない
  Given Stop を行わず turn が最後まで応答した
  Then turn は正常完了（completed）として扱われる
  And 通常完了挙動は従来どおりである

Scenario: stale timeout 挙動を壊さない
  Given turn が応答中のまま stale timeout 条件に達した
  Then 既存の stale timeout 挙動が従来どおり適用される

Scenario: workflow step 実行挙動を壊さない
  Given workflow step が実行されている
  Then 既存の workflow step 実行挙動が従来どおり適用される
```

---

## Open Questions

なし。
