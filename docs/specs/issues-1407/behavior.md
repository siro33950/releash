# Behavior

関連: #1407 / requirements.md / milestone 84「Agentチャット安定化」Phase 0（監査項目 SD-1 / OB-8、ライフサイクル I9）

本書は、agent session の backend resume 失敗（Claude の resume mismatch / Codex の backend thread 消失）に対する回復挙動を、外部から観測可能なビジネスルールとして定義する。内部の型名・関数・トランザクション実装（`recovery_id` の具体表現、event の append 順序の内部機序、atomic commit の実装手段等）は behavior の対象外とし、design.md で扱う。ここでは「利用者・チャット・セッション状態から観測できる結果」に絞る。

## 仮定

- （仮定）本書が対象とする「backend session の消失」とは、Claude の resume mismatch（CLI が別 session_id で init を返す）と、Codex の backend thread 消失（codex home 変更・rollout ファイル削除・thread GC 等により死んだ thread id への resume が失敗する）の双方を指す。両者は同一の回復経路（I9）で扱う。
- （仮定）回復通知の文言は日本語で「backend セッションを作り直したため文脈は引き継がれません」を基準とする。通知は既存の Notice / Error part の枠組みに載せる。通知経路（Notice）が未整備の場合は Error part を暫定手段とする（requirements R3、非スコープ「通知の恒久的 UI 刷新」）。
- （仮定）共有 event 型（`BackendSessionRecoveryStarted` / `SessionConfigurationReactivated` / `SessionGoalReactivated` / `BackendSessionRecoveryCompleted` / `GoalReactivationOutcome`）の定義自体は #1397 等が主管し、本 issue は SD-1 / OB-8 の回復経路で配線・利用する範囲に限る。behavior ではこれらの内部名を用いず、観測可能な結果（回復の相関・部分適用の非公開・Goal 再記録の網羅）として表現する。
- （仮定）ロジックは全て Rust（usecase / domain / infrastructure）に置く。frontend は通知の表示のみを担い、回復判断やトランザクション制御を持たない。
- （仮定）「回復中の block」とは、回復トランザクションの確定・公開が完了するまで、そのセッションの configuration / Goal 変更が回復処理と競合しないよう保留される状態を指す。回復完了後に通常操作へ戻る。
- （仮定）「thread 消失の模擬」は外部プロセスを実行せず、codex home / rollout ファイル差し替え等でテスト内から再現する（外部プロセスをテストで実行しない方針）。

---

## Feature: backend resume 失敗の統一回復とユーザー通知

backend session の resume 失敗（Claude の mismatch・Codex の thread 消失）を、backend 非対称なく同一の回復経路に統一する。回復後もセッションは恒久死せず会話を継続でき、回復が起きたことを利用者へ通知する。

### Background

```gherkin
Background:
  Given agent session が backend セッションを確立して会話が進行している
  And そのセッションには resume 用の backend session id（resume metadata）が保存されている
```

---

### Rule: Codex の backend thread 消失後もセッションは恒久死しない（R1 / R2 / AC1 / AC4）

Codex の backend thread が確立後に消失しても、次の送信でセッションが Error のまま恒久死せず、新規 backend セッションを再確立して会話を継続できる。死んだ thread id への resume を送信のたびに繰り返す状態に陥らない。

```gherkin
Scenario: 確立後に thread が消失しても次の送信で復活する
  Given Codex セッションで backend thread が確立済みである
  And その後 backend thread が消失している（死んだ thread id への resume が失敗する状態）
  When 利用者が次のメッセージを送信する
  Then resume 失敗が backend session の消失として回復経路に到達する
  And セッションは保存済み resume metadata を破棄して新規 backend セッションを確立する
  And 会話は新規セッションで継続でき、送信内容は処理される
  And セッションは Error のまま留まらない

Scenario: 復活後の後続送信は死んだ thread を再利用しない
  Given 直前の送信で backend session が作り直されている
  When 利用者がさらにメッセージを送信する
  Then 送信は新しい backend セッションに対して行われる
  And 死んだ thread id への resume は繰り返されない

Scenario: 従来は恒久死していた（リグレッション対象の否定挙動）
  Given Codex セッションで backend thread が消失している
  When 利用者が繰り返しメッセージを送信する
  Then 生の JSON-RPC エラーを毎回表示して復旧不能に陥ることはない
```

---

### Rule: backend セッション再確立時にユーザーへ通知する（R3 / R4 / AC1 / AC2）

backend セッションが作り直された場合、Claude / Codex いずれでも、利用者へ「backend セッションを作り直したため文脈は引き継がれない」旨を通知する。Claude の resume mismatch による復旧も、従来の無言復旧をやめて同じ通知を出す。

```gherkin
Scenario: Codex の thread 消失回復で通知が出る
  Given Codex の backend thread 消失から回復して新規セッションを確立した
  Then チャットに「backend セッションを作り直したため文脈は引き継がれません」旨の通知が表示される

Scenario: Claude の resume mismatch 回復で通知が出る
  Given Claude が resume mismatch（別 session_id での init）を検知した
  When runtime が resume metadata を破棄して新規セッションで自動再開する
  Then チャットに Codex と同じ回復通知が表示される
  And 文脈が静かに消える（通知なしの復旧）ことはない

Scenario Outline: 通知は既存の Notice / Error part の枠組みに載る
  Given 回復通知を出す必要がある
  When 通知経路の整備状況が "<state>" である
  Then 通知は "<part>" として表示される

  Examples:
    | state          | part        |
    | Notice 経路あり | Notice part |
    | Notice 経路なし | Error part  |
```

---

### Rule: resume mismatch のリトライ turn は editor_context を保持する（R5 / AC3）

resume mismatch により requeue・リトライされる turn は、元の turn の `editor_context` を保持する。Codex のワイヤ送信（additionalContext）でも、Claude の system prompt 再構築でも、リトライ turn からエディタ状態（アクティブファイル・選択範囲）が失われない。`mentions` / `images` と同様に `editor_context` も保全される。

```gherkin
Scenario: リトライ turn で editor_context が保持される
  Given 実行中の turn が editor_context（アクティブファイル・選択範囲）を持つ
  When resume mismatch により当該 turn が requeue されリトライされる
  Then リトライ turn の editor_context は元の turn と同一である
  And editor_context が None に脱落しない

Scenario: Codex のリトライではエディタ状態がワイヤ送信される
  Given Codex セッションで editor_context を持つ turn が resume mismatch でリトライされる
  When リトライ turn が backend へ送信される
  Then editor_context が additionalContext として送信される

Scenario: Claude のリトライでは system prompt にエディタ状態が反映される
  Given Claude セッションで editor_context を持つ turn が resume mismatch でリトライされる
  When リトライ turn の system prompt が再構築される
  Then エディタ状態が system prompt に含まれる

Scenario: mentions / images と editor_context の保全は対称である
  Given turn が mentions・images・editor_context を持つ
  When resume mismatch により turn がリトライされる
  Then mentions・images・editor_context のいずれも保持され、editor_context だけが落ちることはない
```

---

### Rule: 回復は相関付けられ、部分適用状態が公開されない（R6 / AC4 / AC5）

回復は同一の回復単位として相関付けられ、resume metadata の破棄・回復開始・configuration/Goal の復旧・回復完了が定義された境界で確定される。回復の途中経過（部分適用状態）は公開されず、確定して初めてセッションに反映・公開される。`BackendSessionCleared` は dead code ではなく、Claude / Codex 双方の回復経路から到達可能である。

```gherkin
Scenario: 回復開始から完了までが一つの回復として相関付けられる
  Given backend session の消失が検知された
  When 回復が実行される
  Then resume metadata の破棄・回復開始・configuration/Goal 復旧・回復完了が同一の回復として相関付けられる

Scenario: 回復の中途状態は公開されない
  Given 回復が進行中である
  When 回復がまだ確定していない
  Then 部分適用状態（回復途中の configuration / Goal / セッション状態）は公開されない
  And 回復が確定して初めて新しい状態が公開される

Scenario: 回復完了は開始と復旧の後に確定・公開される
  Given 回復が開始され configuration / Goal の復旧が行われた
  When 回復完了が確定される
  Then 公開順序は「回復開始 → configuration/Goal 復旧 → 回復完了」である

Scenario: 回復中は configuration / Goal 変更が保留される
  Given セッションが回復中である
  When configuration または Goal の変更が要求される
  Then その変更は回復の確定まで保留され、回復処理と競合しない

Scenario: BackendSessionCleared が production 経路から到達可能である
  Given backend session の消失が検知された
  When 回復経路が実行される
  Then backend session の消失イベントは受信者のいないチャネルで drop されず、回復処理に到達する
```

---

### Rule: configuration / Goal の再活性化は無検証流用しない（R6 / R7 / AC5）

新しい provider session に対して、旧 session の effective 値をそのまま無検証で流用しない。Goal は None / terminal / unchanged / restored のいずれの場合でも必ず再活性化として記録される（いずれの結末も網羅して記録し、記録漏れがない）。

```gherkin
Scenario: 新 provider session の実効値を旧値の無検証流用にしない
  Given 回復により新しい provider session が確立される
  When configuration が再活性化される
  Then 新 provider session の実効値は旧 effective 値の無検証流用ではない

Scenario Outline: Goal はいずれの結末でも必ず再活性化として記録される
  Given 回復前の Goal の状態が "<goal_state>" である
  When 回復で Goal の再活性化が行われる
  Then Goal 再活性化が結末 "<outcome>" として記録される
  And 記録が省略されることはない

  Examples:
    | goal_state    | outcome   |
    | None          | none      |
    | terminal      | terminal  |
    | 変化なし       | unchanged |
    | 復元された      | restored  |

Scenario: Goal 復元が turn を開始する戦略のときは turn 開始まで含めて確定する
  Given Goal の復元戦略が「turn を開始する」である
  When 回復が確定される
  Then turn 開始（evidence 付き）も回復の確定に含まれる
  And 回復確定前に流れる early stream は buffer される
```

---

### Rule: 回復・保全挙動は統合テストで検証される（R8 / AC6）

回復・editor_context 保全の挙動を検証する統合テストが追加され、既存テストと共に green である。thread 消失の模擬は外部プロセスを実行せず、codex home / rollout ファイル差し替え等でテスト内から再現する。

```gherkin
Scenario: Codex thread 消失からの復活と通知を検証する統合テスト
  Given codex home / rollout ファイルの差し替えで backend thread 消失を模擬する
  When 次の送信を行う
  Then セッションが復活し、回復通知が出ることが検証される

Scenario: Claude resume mismatch の通知を検証する
  Given Claude が resume mismatch を検知する状況を作る
  When 復旧が実行される
  Then 回復通知が出ることが検証される

Scenario: リトライ turn の editor_context 保全を検証する
  Given editor_context を持つ turn が resume mismatch でリトライされる状況を作る
  When リトライが行われる
  Then editor_context が保持されることが検証される
```

---

## Open Questions

なし。
