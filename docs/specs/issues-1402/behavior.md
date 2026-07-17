# Behavior

関連: #1402 / requirements.md / マイルストーン84 `agent-chat-ideal-lifecycle.md`（不変条件 I5、判断 L-D2 / L-D5）

本書は Issue #1402「L1: 停止（interrupt）の信頼性保証」で保証される、外部から観測可能な振る舞いを定義する。対象は turn の停止操作と、停止後の pending queue の扱いである。実装経路（event 名・タイマ実装・層構成）は behavior の対象外とし、design.md で扱う。

本書のシナリオは、backend（Claude / Codex）の別に関わらず同一の保証を要求する。停止の信頼性差（Claude だけが猶予タイマを持つ非対称）を解消し、両 backend で同じ振る舞いに揃えることが本 Issue の主眼である。

## 仮定

- **A1**: 「turn の phase」は `StartingTurn`（送信直後・backend ack 前）/ `Streaming` / `WaitingPermission` / `Interrupting` を指す（lifecycle 状態機械に準拠）。本書の Stop はこれら全 phase で受理される。
- **A2**: 「最悪 10 秒」は強制終端の猶予上限であり、backend が停止を確認できればそれより早く終端してよい。10 秒は絶対レイテンシではなく上限として扱う（requirements A3 / L-D2）。
- **A3**: `terminal 状態` は turn が `Interrupted`（`Timeout` 含む）で finalize され、UI が Idle に戻り、停止スピナー・操作可能な permission ダイアログが残らない状態を指す。
- **A4**: pending queue 本体の永続化は L3（#1404）の対象だが、queue の `paused` は本 Issue で event log から復元する。本書は「停止直後および再起動後に自動実行しない」「明示操作でのみ再開する」を保証する。
- **A5**: queue 再開の明示操作は暫定として既存 UI（queue chips 相当）で受ける（requirements A4）。本書は再開の「経路の性質」（ユーザー明示操作に限る）を保証し、専用 UI の作り込みは対象外とする。
- **A6**: 「無損失」とは、停止操作によって入力欄のテキスト・添付・pending queue の各メッセージが失われないことを指す（削除ではなく保持）。
- **A7**: Stop 受理の durable commit は event log への append を指す（requirements A2）。本書は「終端前に crash しても再起動後に queue が自動 drain されない」という観測結果として検証する。

## Feature: turn 停止（interrupt）の信頼性保証

### Background

```gherkin
Background:
  Given Agent チャットのセッションが 1 つ開いている
  And そのセッションの turn が進行中で停止ボタンが操作可能である
```

---

### Rule: 停止操作はどの phase でも常に受理される（I5 / R1）

Stop は turn の phase（`StartingTurn` / `Streaming` / `WaitingPermission` / `Interrupting`）に関わらず受理される。backend への interrupt 送出が不可能・無応答でも、猶予後に必ず terminal 状態へ着地する。

```gherkin
Scenario Outline: どの phase の Stop も受理され turn が終端する
  Given turn の phase が "<phase>" である
  When 利用者が停止ボタンを押す
  Then 停止操作は受理される
  And turn は最悪 10 秒以内に terminal 状態（Interrupted）で終端する
  And 停止スピナーや操作可能な permission ダイアログが残らない

  Examples:
    | phase            |
    | StartingTurn     |
    | Streaming        |
    | WaitingPermission|
    | Interrupting     |

Scenario: backend が無応答でも猶予後に強制終端する
  Given turn が進行中である
  And backend が interrupt に応答しない（ハングしている）
  When 利用者が停止ボタンを押す
  Then turn は最悪 10 秒以内に Interrupted{Timeout} として強制終端する
  And セッションが無期限にロックされたままにならない
```

---

### Rule: Codex の turn_id 未取得ウィンドウでの停止予約（OB-1 / SD-2 / R2）

送信直後、backend の turn 開始通知（turn_id 取得）前に押された Stop を無言 no-op にせず、予約として保持し、turn 開始通知の受信時に即 interrupt を送出する。

```gherkin
Scenario: 送信直後（turn 開始通知前）の Stop が握りつぶされない
  Given turn を送信した直後で、backend からの turn 開始通知をまだ受信していない
  When 利用者が停止ボタンを押す
  Then 停止操作は無言で握りつぶされず受理される
  And turn は最悪 10 秒以内に terminal 状態で終端する

Scenario: turn 開始通知の受信で予約された停止が送出される
  Given 送信直後（turn 開始通知前）に停止操作が受理され予約されている
  When backend から turn 開始通知（turn_id 取得）を受信する
  Then 予約された interrupt が即座に backend へ送出される
  And 予約された停止が通常の turn 開始として実行されない

Scenario: StartingTurn の late ack が通常の turn 開始に流れない
  Given StartingTurn（backend ack 前）で停止操作が受理されている
  When backend の ack が遅れて到着し provider turn が取得できる
  Then その turn は通常の turn として継続せず、interrupt / finalize される
  And ack / interrupt の結果が不明な場合は同一 request の TurnStart 照合（reconciliation）に畳まれる
  And いずれの場合も turn は Interrupted で終端する
```

---

### Rule: 停止後に pending queue を自動実行しない（OB-5 / L-D5 / R5）

interrupt 時、pending queue を無条件 drain せず paused にする。停止直後に次の queue メッセージが自動実行開始されない。再開はユーザーの明示操作でのみ行う。

```gherkin
Scenario: 停止直後に次の queue メッセージが自動実行されない
  Given pending queue にメッセージが 1 件以上積まれている
  And turn が進行中である
  When 利用者が停止ボタンを押して turn を終端させる
  Then queue は paused になる
  And 次の queue メッセージは自動的に次の turn として実行開始されない
  And queue の各メッセージは失われず保持される

Scenario: 再開はユーザーの明示操作でのみ行われる
  Given 停止により queue が paused になっている
  When 利用者が明示的に再開操作を行う
  Then paused の queue が再開され、メッセージが実行され得る

Scenario: 停止・backend・frontend の停止経路は queue を勝手に再開しない
  Given 停止により queue が paused になっている
  When interrupt / backend / frontend の停止経路のみが進行し、利用者は再開操作をしない
  Then queue は paused のまま維持され、自動的に再開されない
```

---

### Rule: 停止受理は crash 耐性を持つ（設計ゲート追補 / R4）

Stop 受理を backend I/O の前に durable に確定するため、終端前に crash しても再起動後に queue が自動 drain されない。既に paused の場合の再受理は冪等である。

```gherkin
Scenario: 終端前に crash しても queue が自動 drain されない
  Given turn が進行中で pending queue にメッセージが積まれている
  And 利用者が停止ボタンを押して停止が受理された
  When turn が terminal に着地する前にアプリが crash する
  When アプリを再起動してセッションを開き直す
  Then queue は paused のままで、queue メッセージが自動実行開始されない

Scenario: 既に paused の状態での停止受理は冪等である
  Given queue が既に paused である
  When 停止操作が再度受理される
  Then queue の状態は paused のまま変わらず、二重の副作用を生じない
```

---

### Rule: 停止中の再押下を握りつぶさない（OB-1 frontend / R6）

interrupt 中（`Interrupting` 表示中）の停止ボタン再押下を握りつぶさず、強制終端要求として backend へ伝える。停止が握りつぶされたことで turn の自然終了まで再送手段が失われる状態を作らない。

```gherkin
Scenario: 停止中の再押下が強制終端要求として受け付けられる
  Given 一度停止操作を行い、UI が interrupt 中（Interrupting）の表示になっている
  When 利用者が停止ボタンを再度押す
  Then 再押下は無視されず、強制終端要求として backend へ伝わる

Scenario: 停止が効かないまま再送手段が失われない
  Given 最初の停止操作が backend へ届かず turn が終端していない
  When 利用者が停止を再送しようとする
  Then 停止ボタンは turn の自然終了を待たずに再度操作可能である
```

---

### Rule: シナリオ表「ユーザー Stop」の統合保証（受け入れ基準）

lifecycle シナリオ表「ユーザー Stop」の保証（最悪 10 秒で Idle・queue は paused・入力欄 / queue は無損失）を単一の経路で満たす。

```gherkin
Scenario: ユーザー Stop の一括保証
  Given Codex セッションで turn を送信した直後である
  And 入力欄にテキストが残り、pending queue にメッセージが積まれている
  When 利用者が停止ボタンを押す
  Then turn は最悪 10 秒以内に Idle（terminal）へ着地する
  And queue は paused になり、queue メッセージは自動実行されない
  And 入力欄のテキスト・添付と queue の各メッセージは失われない
```

---

## 主要な境界条件・異常系

- **境界（猶予上限）**: backend の停止確認が取れない場合の終端は最悪 10 秒。backend が早く応答すればより早く終端してよい（上限であって固定遅延ではない）。
- **境界（turn_id 未取得ウィンドウ）**: 送信直後〜turn 開始通知受信までの窓で押された Stop も予約として保持され、no-op にならない。
- **異常（backend ハング）**: interrupt 応答が返らない場合も Interrupted{Timeout} で強制終端し、セッションの無期限ロックを作らない。
- **異常（終端前 crash）**: durable 確定済みのため再起動後も queue は paused を維持し、自動 drain しない。
- **異常（late ack で結果不明）**: ack / interrupt 結果が不明な場合は TurnStart reconciliation へ畳み、通常 turn として継続させない。
- **冪等**: 既に paused の queue に対する停止受理は状態を変えず、副作用を重複させない。

## 非スコープ（本書が保証しないこと）

- pending queue 本体の永続化・取消の完全化（L3 #1404）。pause/resume 状態の永続化と復元は本書の保証に含む。
- queue 再開専用 UI の作り込み（暫定は既存 UI）。
- stalled / steer 送信時のユーザー入力無損失（L2 #1403、OB-2）。
- close / quit 時の finalize（L4 #1405）。
- 正規化語彙・schema 変更（Phase 1 以降）。
- stall watchdog の挙動変更（L9 #1410）。

## Open Questions

なし。
