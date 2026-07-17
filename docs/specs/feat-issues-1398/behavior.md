# Behavior

Agent チャットで発生する 2 つのエラー経路 — turn 実行中の CLI プロセス死（crash）と、Idle 中の backend プロセス死（Fatal）— を、発生時点で live UI に即時着地させ、reload 後の表示と一致させる振る舞いを定義する。

対象は監査項目 FE-2（high）と RT-6（low）の 2 経路に限る。語彙は現行の Error part / session state に閉じ、新規語彙（Notice 等）は導入しない。timeout 経路・permission dialog・streaming hydration 等は対象外とする。

ここで定義するのは外部から観測可能な振る舞いに限る。emit 経路の実装方式・Error part の紐付け先・read model の具体構造は `design.md` で確定する。

## Feature: turn 実行中の crash が live で chat panel に着地する

```gherkin
Feature: turn 中 crash の live 着地
  turn 実行中に CLI プロセスが死んだとき、その失敗は reload を待たず
  その場で chat panel に Error 表示として現れる。
  「生成中に spinner が消えて agent が勝手にやめたように見える」状態を禁止する。

  Background:
    Given agent session が turn を実行中である
    And chat panel は生成中の状態を表示している

  Rule: crash 発生時点で Error 表示が live に着地する

    Scenario: turn 中に CLI プロセスが死ぬと即座に Error block が現れる
      Given turn の完了・中断を示す明示的終端イベントをまだ受信していない
      When 実行中の CLI プロセスが死ぬ
      Then chat panel に Error block が reload なしでその場に現れる
      And 生成中を示す表示（spinner 等）は Error 表示に置き換わる
      And 失敗した tool 呼び出しの結果も同じ chat panel に現れる

    Scenario: crash 前に確定した出力は Error block と併存する
      Given turn の途中まで応答が chat panel に表示されている
      When 実行中の CLI プロセスが死ぬ
      Then それまでに表示された出力は消えない
      And その後ろに Error block が続けて現れる
```

## Feature: Idle 中の Fatal がその理由付きで記録され live に着地する

```gherkin
Feature: Idle 中 Fatal の理由記録と live 着地
  active な turn / message が無い Idle 状態で backend プロセスが死んだとき、
  その理由が durable に記録され、chat panel に Error block として live 着地し、
  session と badge から理由を読める。「痕跡ゼロ」状態を禁止する。

  Background:
    Given agent session が Idle 状態である
    And 実行中の turn も未完了の message も存在しない

  Rule: Idle 中の Fatal は理由付きで durable に着地する

    Scenario: Idle 中に backend プロセスが死ぬと理由が記録される
      When Idle 状態で backend プロセスが死ぬ
      Then session state は Error になる
      And その Error の理由（何が起きたか）が durable に記録される
      And 理由は session / get_session から読み出せる

    Scenario: Idle 中の Fatal が chat panel に live 着地する
      When Idle 状態で backend プロセスが死ぬ
      Then chat panel に Error block が reload なしでその場に現れる
      And Error block はその Fatal の理由を含む

  Rule: Fatal 記録後の後続 event で理由が消えない

    Scenario: Fatal 記録後に別の event が append されても理由が残る
      Given Idle 中の Fatal が理由付きで記録されている
      When その後に別の event が append され、状態が再投影される
      Then session state は依然として Error である
      And Error の理由は失われず、引き続き session / get_session から読める
```

## Feature: session バッジが Error の理由を表示する

```gherkin
Feature: session バッジの Error 理由表示
  session バッジ（一覧・タブ）の Error 状態は、その理由を tooltip で表示する。
  理由の source of truth は backend の read model に置き、reload 後も残る。

  Background:
    Given ある agent session が Error 状態である
    And その Error には理由（最後の crash / Fatal の要約）が記録されている

  Rule: Error バッジは理由 tooltip を持ち reload 後も残る

    Scenario: Error バッジに理由 tooltip が表示される
      When session バッジを一覧またはタブで確認する
      Then バッジは Error 状態を示す
      And バッジの tooltip はその Error の理由を表示する

    Scenario: reload 後もバッジの理由が残る
      Given session バッジが Error の理由 tooltip を表示している
      When アプリを reload して session を読み直す
      Then バッジは依然として Error 状態を示す
      And tooltip は同じ理由を表示する
```

## Feature: live と reload 後の表示が一致する

```gherkin
Feature: live と reload の表示等価
  crash（turn 中）・Idle-Fatal（Idle 中）いずれのエラーも、
  発生時点の live 表示と、get_session 再読込後の表示が一致する。

  Rule: エラー表示は live と reload 後で等価である

    Scenario Outline: 各エラー経路で live と reload 後の表示が一致する
      Given <エラー経路> によって chat panel に Error 表示が live 着地している
      When 同じ session を reload して get_session から読み直す
      Then chat panel の Error 表示は live 着地時と同一である
      And session state と Error の理由も live と reload 後で一致する

      Examples:
        | エラー経路              |
        | turn 中の CLI プロセス死（crash） |
        | Idle 中の backend プロセス死（Fatal） |
```

## 仮定

- A1: crash の live 着地は、既存の transient 経路（streaming delta / `agent-session-state-changed`）を拡張して行い、新規 event / DTO 種別の追加は最小限に留める。durable 側の Error part 合成（`finalize_turn` → projector）は既存挙動を維持し、live へ同一情報を届けることを主眼とする。
- A2: RT-6 の理由保持は、`SessionState::Error` に付随する理由を read model / `get_session` から復元できる形で持たせる。projection 上書き対策は、理由を durable event 由来にして再投影で復元可能にすることで満たす。
- A3: badge 理由 tooltip は frontend の表示のみを担い、理由の source of truth は backend read model（`get_session` / session summary）に置く。frontend は受信データの表示に徹する。
- A4: 「live と reload 後の表示が一致」は crash（turn 中）と Fatal（Idle）の両経路を対象とし、両者を自動テストで固定する。live emit・durable 記録・reload 一致は Rust 側テストで、frontend の Error 表示変換は frontend テストで固定する。
- A5: 対象は crash と Idle-Fatal の 2 経路のみ。`InterruptReason::Timeout` は現コードに生成元が無いため対象外とし、現行の Error part / session state 語彙に閉じて新規語彙（Notice 等）を導入しない。
- A6: active turn / message が無い Idle 状態での Error part の紐付け先は `design.md` で具体化する。本振る舞いでは「chat panel に理由付き Error block が live 着地し reload と一致する」ことを外部観測可能なルールとして定める。

## Open Questions

なし（RT-6 の着地先は「chat Error block も着地」で確定。active turn / message が無い状態での Error part 紐付け方式は design node で具体化する）。
