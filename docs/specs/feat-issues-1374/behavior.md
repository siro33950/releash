# Behavior

無出力 timeout（stale watchdog）を turn 完了・中断判定から切り離し、backend の明示的終端イベントを turn 終端の正系とする振る舞いを定義する。

無出力 timeout は、非破壊な観測・recovery・介入点提示の補助 signal として扱う。

## Feature: turn 終端は backend の明示的終端イベントを正系とする

```gherkin
Feature: turn 終端判定の正系
  agent session の turn 完了・失敗・中断は、backend 由来の明示的終端イベントで確定する。
  無出力時間の経過は turn 終端の判定根拠にしない。

  Background:
    Given agent session が実行中である
    And session は streaming 中で backend の応答を待っている

  Rule: turn の完了・中断は backend の明示的終端イベントでのみ確定する

    Scenario: backend の明示的終端イベントで turn が完了する
      Given backend がまだ終端イベントを発行していない
      When backend の明示的終端イベントを受信する
      Then turn は正しく完了・中断として確定する
      And workflow runtime は確定した turn 終端を判断材料として次の制御へ進める

    Scenario: 無出力時間の経過だけでは turn が終端しない
      Given backend の明示的終端イベントを受信していない
      When 無出力 timeout の閾値へ到達する
      Then turn は完了・失敗・中断のいずれにも確定しない
      And session は backend の終端イベントを引き続き待てる状態のまま残る
```

## Feature: 無出力 timeout は非破壊な補助 signal として扱う

```gherkin
Feature: 無出力 timeout の非破壊処理
  無出力 timeout の到達は、backend 終端イベントの欠落可能性、transport / stream / backend process の異常、
  または長時間無出力だが処理継続中の状態を観測する signal として扱う。到達だけで session / runtime を破棄しない。

  Background:
    Given agent session が streaming 中である
    And backend の明示的終端イベントを受信していない

  Rule: 無出力 timeout 到達で session / runtime を破棄しない

    Scenario: 無出力 timeout 到達でもエラー中断や runtime close が起きない
      When 無出力 timeout の閾値（既定 180 秒 / 設定値）へ到達する
      Then 無出力を理由とするエラー中断メッセージで turn が終端されない
      And runtime は破棄されず継続可能な状態に保たれる
      And backend-owned state は保持される

    Scenario: 生きているが無出力の区間で turn がエラー終端されない
      Given session は backend 内部で処理継続中だが出力を発していない
      # 例: reasoning 中、ToolUse part 未到着、KeepAlive 途絶
      When 無出力 timeout の閾値へ到達する
      Then turn はエラー終端されない
      And session は処理継続を待てる状態のまま残る

  Rule: 無出力 timeout 到達時の自動処理は非破壊 recovery に限る

    Scenario: 無出力 timeout 到達時に非破壊 recovery と介入点提示のみを行う
      When 無出力 timeout の閾値へ到達する
      Then session / runtime を破棄する自動処理は行われない
      And 許容される自動処理は stream / transport の再接続と backend-owned state の再読込に限られる
      And 利用者・workflow は retry / continue / abort を選べる介入点を提示される

  Rule: 自動継続・再接続には暴走防止の上限がある

    Scenario: 上限内での自動継続・再接続
      Given 自動継続・再接続の試行が上限に達していない
      When 無出力 timeout の閾値へ到達する
      Then 非破壊 recovery が試みられる

    Scenario: 上限到達で自動処理を止め介入点へ委ねる
      Given 自動継続・再接続の試行が上限（回数・時間など）に達している
      When 無出力 timeout の閾値へ再度到達する
      Then それ以上の自動継続・再接続は行われない
      And 利用者・workflow 介入点へ委ねられる
      And session / runtime は破棄されず継続可能な状態に保たれる
```

## Feature: 別経路の終端は従来どおり尊重する

```gherkin
Feature: 無出力 timeout と独立した終端経路
  user 明示 cancel、workflow の wall-clock / run timeout、backend の明示 terminal / fatal event、
  tool 固有 timeout は、無出力 timeout とは独立した経路として引き続き turn / session を終端できる。

  Rule: 無出力 timeout の変更は他の終端経路に影響しない

    Scenario Outline: 独立した終端経路は従来どおり turn / session を終端する
      Given agent session が実行中である
      When <終端契機> が発生する
      Then turn / session は従来どおり終端する

      Examples:
        | 終端契機                          |
        | user 明示 cancel                  |
        | workflow の wall-clock / run timeout |
        | backend の明示 terminal / fatal event |
        | tool 固有 timeout                 |
```

## 仮定

- 現状コードで確認した挙動を現行仕様として扱う。
  - `spawn_stale_watchdog_task` が無出力 timeout 成立時に `TurnResult::Interrupted { reason: Timeout }` を合成し、`runtime.interrupt()` → grace 待機 → `runtime.close()` まで進める。
  - 基準 timeout は既定 180 秒、上限 1800 秒。ToolResult 未着の ToolUse が残る間のみ上限まで延長。
  - `last_progress_at` の更新契機は、streaming の domain part 受信、KeepAlive 受信（phase != Idle）、permission 応答での streaming 再開の 3 つに限られる。
- backend の明示的終端イベントは `AgentRuntimeEvent::TurnCompleted` として既に runtime usecase へ届いており、これを完了判定の正系として利用できる。
- `workflow_step_context.stale_timeout_secs` は今後「補助 signal の発火閾値」として意味づけを保つ（廃止しない）。この意味変更方針は後続 Spec で確認する。
- 「非破壊 recovery の具体実装方式」「介入点の提示形態」「暴走防止上限の具体値」は本振る舞いでは外部観測可能なルールに留め、詳細は `design.md` で確定する（requirements の Non-goals に従う）。

## Open Questions

なし（復帰方式の詳細は Non-goals として扱い、`design.md` で確定する）。
