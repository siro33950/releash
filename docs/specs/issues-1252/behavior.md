# Behavior

関連: #1252

requirements.md の要求を、実装詳細を含まない観測可能な振る舞いとして定義する。

## 仮定

- watchdog は `evaluate_turn_liveness` の戻り値（`Some(timeout)` / `None`）でターンを中断するかどうかを決める。`None` が返る限りターンは中断されない。本振る舞い定義では `evaluate_turn_liveness` の判定結果＝「watchdog がターンを中断するか否か」という外部観測可能なルールとして扱う。
- ターン中断時には、対応するエラー文言が利用者に表示される。
- permission timeout の撤去は origin（`Desktop` / `Headless`）に依らず適用する（requirements 案 A）。本振る舞いでは origin による分岐は存在しないものとして記述する。

## Feature: 権限承認待ちはタイムアウトしない

Agent session のターンが権限承認待ち（`WaitingPermission`）にある間は、経過時間に関わらず watchdog によって中断されない。一方、Streaming 中の応答停止（stale）検知は従来どおり維持される。

### Background

```gherkin
Given Agent session のターンが進行している
And watchdog が一定間隔でターンの liveness を評価する
```

### Rule: 権限承認待ちは経過時間で中断されない

```gherkin
Scenario: 承認待ちが従来のタイムアウト閾値を大幅に超過しても中断されない
  Given ターンが権限承認待ち（WaitingPermission）にある
  When 承認待ちのまま、従来の打ち切り閾値を大幅に超える時間が経過する
  Then watchdog はターンを中断しない
  And 「権限承認の待機がタイムアウトしたため中断しました。」のエラーは表示されない

Scenario: 承認待ち中はどれだけ待っても中断されない
  Given ターンが権限承認待ち（WaitingPermission）にある
  When 利用者が承認も拒否もせず待機を続ける
  Then watchdog は経過時間を理由にターンを中断しない

Scenario Outline: origin に依らず承認待ちは中断されない
  Given ターンが権限承認待ち（WaitingPermission）にあり、origin が <origin> である
  When 承認待ちのまま長時間が経過する
  Then watchdog はターンを中断しない

  Examples:
    | origin   |
    | Desktop  |
    | Headless |
```

### Rule: Streaming 中の応答停止検知は従来どおり維持される

```gherkin
Scenario: Streaming 応答が停止閾値を超えて停止すると中断される
  Given ターンが Streaming 中である
  When 応答が stale 検知の閾値（STALE_TIMEOUT_SECS）を超えて更新されない
  Then watchdog はターンを中断する
  And 「Claude 応答が停止したため中断しました。もう一度お試しください。」のエラーが表示される

Scenario: Streaming 応答が閾値内で更新され続ける限り中断されない
  Given ターンが Streaming 中である
  When 応答が stale 検知の閾値を超えない間隔で更新され続ける
  Then watchdog はターンを中断しない
```

### Rule: アイドル状態は中断されない

```gherkin
Scenario: Idle フェーズは liveness 判定で中断されない
  Given ターンが Idle 状態である
  When 時間が経過する
  Then watchdog はターンを中断しない
```

## 主要な境界条件

```gherkin
Scenario: 承認直後に Streaming へ遷移したターンには stale 検知が適用される
  Given ターンが権限承認待ち（WaitingPermission）にあった
  And 利用者が承認した
  When ターンが Streaming に遷移し、応答が stale 閾値を超えて停止する
  Then watchdog はターンを中断する
  And 「Claude 応答が停止したため中断しました。もう一度お試しください。」のエラーが表示される
```

## 非機能・整理に関する振る舞い

これらは Gherkin で表す利用者向け振る舞いではなく、requirements の受け入れ基準（コードベース品質）として満たすべき制約である。design.md / 実装で担保する。

- permission timeout 撤去により未使用となったシンボルがコードベースに残存しないこと（デッドコードを残さない）。
- `cargo clippy -- -D warnings` / `cargo fmt --check` / `cargo test` が通ること。
- 上記の承認待ち無中断・stale 維持の挙動を担保するテストが存在すること。

## Open Questions

なし。
