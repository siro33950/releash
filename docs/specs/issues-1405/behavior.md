# Behavior

関連: #1405 / requirements.md / milestone 84（Agentチャット安定化・Phase 0 / L4）/ 監査 RT-1 / 不変条件 I1（turn 終端保証）

本書は「進行中 turn を持つセッションを、正常な close / backend 切替 / アプリ終了のいずれの経路で閉じても、その turn が必ず終端され、再オープン後に残骸（永久スピナー・永久確認待ち permission・欠落した本文・欠落した terminal event）が残らない」という外部から観測可能な振る舞いを定義する。

内部の実行順序（flush → finalize → close）・drain の待機時間・emit 抑止の解除方式・永続表現の具体形は behavior の対象外とし、design.md で確定する。本書は「再オープン後・event log 上で何が観測できるか」という性質のみを固定する。

## 仮定

- 本書で「終了経路」と呼ぶのは、正常な `close_session`（タブを閉じる）・`set_session_backend`（backend 切替）・`close_all`（アプリ終了）の三経路を指す。プロセスクラッシュ・強制終了時の起動時 dangling turn 回収（`Crash` 理由）は本 Issue の非スコープ（RT-2 / #1407）であり、本書のシナリオ対象外とする。
- 「進行中 turn」とは、`TurnStarted` 済みで terminal event（`TurnCompleted` / `TurnInterrupted`）を未記録の turn を指す。
- 「中断チップ」とは、再オープンした会話ビュー上で当該 turn が中断されたことを示す表示であり、理由が `SessionClosed` であることを示す。表示文言・色・アイコン等の見た目は behavior の対象外とする。
- 「残骸が無い」とは、再オープン後に (a) 永久に回転し続けるツール実行スピナー（background Task group を含む）が無く、(b) 操作しても必ず失敗する確認待ち permission ダイアログが無い状態を指す。
- 「本文が flush 済みまで残る」とは、close 直前までのストリーミング本文（最後の定期スナップショット以降の増分と未 persist の pending parts を含む）が、再オープン後のメッセージに反映されている状態を指す。損失許容窓は現行の定期 flush 間隔（1 秒）に準ずるが、close 時は force flush によりこの窓内の本文まで確実に durable 化される。
- `set_session_backend` は現行 UI 上「空セッション限定」で主経路ではないが、監査 RT-1 が同一経路と指摘するため終端保証の対象に含める。provider 切替に伴う configuration handoff の再設計は行わない。
- frontend は終了経路を薄く invoke するだけであり、interrupt 判断・finalize は Rust（usecase / domain）が所有する（rust-first-logic）。
- write-ahead 済み（`Responding` / `Resolving`）permission の扱いは既存 finalize 経路の規約に従う。本書は未送信 `Pending` の畳み込みのみを新規保証として固定する。

## Feature: 進行中 turn を持つセッションの終了時の turn 終端保証

### Background

```gherkin
Background:
  Given ある Agent セッションで turn が進行中である
  And その turn はストリーミング本文を出力し、ツール実行または permission 要求を含みうる
```

---

### Rule: 進行中 turn を持つ終了経路は turn を必ず終端させる

正常な close / backend 切替 / アプリ終了のいずれの経路でも、進行中 turn を持つセッションを閉じるときは、その turn が中断として終端される。どの経路でも観測結果は同一である。

```gherkin
Scenario Outline: どの終了経路でも進行中 turn は終端される
  Given turn が進行中である
  When 利用者または system が "<経路>" によってセッションを閉じる
  Then その turn は中断理由 SessionClosed で終端される
  And セッションを再オープンしても未終端の turn は残らない

  Examples:
    | 経路              |
    | チャットタブを閉じる |
    | backend を切り替える |
    | アプリを終了する     |
```

```gherkin
Scenario: 進行中 turn が無いセッションの終了は従来どおり
  Given turn が進行中でない
  When セッションを閉じる
  Then 新たな中断 turn は生成されない
  And 既存の会話履歴はそのまま保持される
```

---

### Rule: 終了時に terminal event（SessionClosed）が必ず記録される

進行中 turn を閉じると、その turn の terminal event（中断・理由 `SessionClosed`）が event log に記録される。既存の `SessionClosed` イベントとは別に、turn 単位の terminal event が残る。

```gherkin
Scenario: 閉じられた turn の terminal event が event log に残る
  Given turn が進行中である
  When セッションを閉じる
  Then event log にその turn の中断 terminal event（理由 SessionClosed）が記録される

Scenario: 再オープン後に中断チップ（SessionClosed）が表示される
  Given 進行中 turn を持つセッションを閉じた
  When セッションを再オープンする
  Then その turn には中断チップが表示される
  And 中断チップは理由が SessionClosed であることを示す
```

---

### Rule: 終了前にストリーミング本文が flush 済みまで残る

close 前にストリーミング本文が強制 flush されるため、再オープン後に close 直前までの本文が失われない。損失窓は定期 flush 間隔を超えない。

```gherkin
Scenario: close 直前までのストリーミング本文が再オープン後に残る
  Given turn が最後の定期スナップショット以降にもストリーミング本文を出力している
  When セッションを閉じて再オープンする
  Then close 直前までのストリーミング本文がメッセージに反映されている
  And 最後の定期スナップショットで失われるはずだった増分・未 persist の pending parts も残っている
```

---

### Rule: 未送信 permission は解決済みに畳まれ、確認待ち残骸が残らない

終了時、未送信（`Pending`）の permission 要求は取消（`Cancelled`・効果なし）として畳まれる。再オープン後に、操作しても必ず失敗する確認待ち permission ダイアログは残らない。

```gherkin
Scenario: 未送信 permission は取消に畳まれる
  Given turn に未送信（Pending）の permission 要求がある
  When セッションを閉じて再オープンする
  Then その permission は取消（効果なし）として表示される
  And 操作可能な確認待ち permission ダイアログは残らない

Scenario: 書き込み済み permission は既存規約に従う
  Given turn に write-ahead 済み（Responding / Resolving）の permission がある
  When セッションを閉じて再オープンする
  Then その permission の扱いは既存 finalize 経路の規約に従う
```

---

### Rule: 進行中のツール実行は中断に畳まれ、永久スピナーが残らない

終了時、進行中の ToolCall は中断（`Interrupted`）として畳まれる。background Task group を含め、再オープン後に永久に回転し続けるスピナーが残らない。

```gherkin
Scenario: 進行中 ToolCall は中断に畳まれる
  Given turn に進行中の ToolCall がある
  When セッションを閉じて再オープンする
  Then その ToolCall は中断として表示される
  And 実行中スピナーは残らない

Scenario: background Task group にも永久スピナーが残らない
  Given turn に完了していない background Task group がある
  When セッションを閉じて再オープンする
  Then その Task group に永久スピナーは残らない
```

---

### Rule: 終了中に届く backend の最終イベントを回収する

close 中に backend から届く最終イベント（result / turn completed）は、無言破棄されず、turn の終端に反映される。

```gherkin
Scenario: close と競合して届く最終イベントを取りこぼさない
  Given セッションを閉じる処理の途中で backend が最終イベント（result / turn completed）を送る
  When セッションを閉じる
  Then その最終イベントは破棄されず turn の終端に反映される
```

---

### Rule: アプリ終了→再起動でも終端が保たれる

アプリ終了時に進行中 turn を閉じた場合も、再起動後に同じ終端保証が観測できる。

```gherkin
Scenario: アプリ終了→再起動後も残骸が無い
  Given ストリーミング中の turn があるセッションが開いている
  When アプリを終了し、再起動してセッションを再オープンする
  Then その turn は中断（SessionClosed）で終端されている
  And 中断チップが表示される
  And 永久スピナー・確認待ち permission 残骸が無い
  And close 直前までのストリーミング本文が残っている
```

---

### Rule: SessionClosed は既存の中断理由と互換に共存する

`InterruptReason` への `SessionClosed` 追加は additive であり、既存の中断理由・既存の永続化済みイベントの解釈を壊さない。`SessionClosed` は Crash 理由を上書きしない。

```gherkin
Scenario: 既存の中断理由が引き続き解釈できる
  Given SessionClosed 追加前に記録された既存の中断 terminal event がある
  When その event log を読み込む
  Then 既存の中断理由はそのまま解釈される
  And SessionClosed の追加によって解釈が変わらない

Scenario: SessionClosed は Crash 理由を上書きしない
  Given ある turn がクラッシュ回収（Crash）の対象である
  When 正常な終了経路が関与しても
  Then その turn の中断理由は SessionClosed で上書きされない
```

---

## Open Questions

なし。
