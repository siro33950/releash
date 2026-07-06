# Behavior

`requirements.md` の R1〜R11 を、実装経路に依存しない外部観測可能な振る舞いとして定義する。
用語:

- **permission request**: agent が人間の判断を求める checkpoint（Claude の `AskUserQuestion`、Codex の `requestApproval` / `item/tool/requestUserInput`）。
- **回答待ち (WaitingPermission)**: backend runtime が permission request を受けて回答を待っている状態。source of truth は Rust 側 runtime / read model。
- **permission dialog**: 回答待ちの permission request をユーザーへ提示し回答を受け付ける UI。

## 仮定

- backend が回答待ち中に保持する pending permission の情報（request id / body / tool 名など）は、permission dialog の表示と回答送信に必要な内容を過不足なく供給できる。転送 shape の具体は design.md で決める。
- 「同一 request id の permission がメッセージ側に無いときだけ pending を描画する」規則で二重表示を避けられる。
- reload で復元できるという性質（R5）を満たせば、その実現手段（即時投影 / 復元合成のいずれか）は問わない。design.md で決める。
- 本 Spec が扱うのは「backend は回答待ちだが UI に dialog が無い」不可視停止であり、provider → 回答待ちへの変換自体は正常であるという Issue 調査結論を前提とする。

```gherkin
Feature: 回答待ちの permission checkpoint を UI が確実に表示・復元できる

  agent session で permission request が発生し backend が回答待ちに入ったとき、
  ユーザーがその checkpoint を必ず観測でき、回答できることを保証する。
  pending permission の source of truth は Rust 側 runtime / read model にあり、
  frontend は backend が返した pending permission を表示するだけに留める。

  Background:
    Given agent session が実行中である
    And permission request が発生して backend runtime が回答待ちに入っている
    And backend runtime は回答待ちの pending permission を保持している

  Rule: backend が回答待ちなら UI は必ず permission dialog を表示できる

    Scenario: 通常の streaming 経路で permission が届く
      Given permission request が message にも届いている
      When ユーザーがその session を表示している
      Then permission dialog が表示される
      And ユーザーは回答できる

    Scenario: streaming 経路で permission が message に届かなかった
      Given permission request が message には届いていない
      But backend runtime は回答待ちの pending permission を保持している
      When ユーザーがその session を表示している
      Then backend が保持する pending permission を元に permission dialog が表示される
      And ユーザーは回答できる

    Scenario: emit 抑止中に permission request が発生した
      Given streaming の emit が抑止されている状態で permission request が発生した
      And permission は message には届いていない
      When ユーザーがその session を表示している
      Then backend が保持する pending permission を元に permission dialog が表示される
      And ユーザーは不可視停止せずに回答できる

  Rule: permission dialog を二重表示しない

    Scenario: message 側と pending 側の双方に同一 request が存在する
      Given 同一 request id の permission が message にも pending にも存在する
      When ユーザーがその session を表示している
      Then permission dialog は 1 つだけ表示される

  Rule: session を離れて戻っても回答待ちが復元される

    Scenario Outline: transient な streaming を失った後に session を開き直す
      Given permission request の後に <離脱> が起きて streaming を受け取れなくなった
      When ユーザーが同じ session を <再表示>
      Then backend が保持する pending permission を元に permission dialog が復元表示される
      And ユーザーは回答できる

      Examples:
        | 離脱           | 再表示           |
        | session reload | reload 後に開く   |
        | tab 移動       | tab を戻す       |
        | 後から open    | 初めて開く       |

    Scenario: 復元した permission に回答すると回答待ちが解消する
      Given session 再表示で pending permission から dialog が復元表示されている
      When ユーザーが回答する
      Then backend runtime の回答待ちが解消する
      And permission dialog が閉じる

  Rule: 未解決 permission の finalize は従来どおり cancelled になる

    Scenario: 回答されないまま turn が finalize される
      Given 回答待ちの permission request が未回答のまま残っている
      When その turn が finalize される
      Then 未解決 permission は cancelled として閉じられる
      And この既存挙動は本変更で回帰しない

  Rule: pending permission の所有は backend にあり frontend は表示に留まる

    Scenario: frontend は backend が返した pending permission を保持して表示するだけ
      Given backend が pending permission を返している
      When frontend がそれを取り込む
      Then frontend は backend が返した pending permission をそのまま表示する
      And frontend は pending permission を再計算・full-retention 保持しない

  Rule: 取りこぼしと回答待ち停滞が診断可能である

    Scenario: 対象 message 未存在で streaming delta が捨てられた
      Given 対象 message が存在しないため streaming delta が適用されず捨てられた
      When その delta が破棄される
      Then 取りこぼしを診断できる warn が出力される

    Scenario: 回答待ちが長期化し visible な dialog が無い
      Given backend が回答待ちのまま一定時間が経過した
      And その間 visible な permission dialog が存在しない
      When 停滞が検知される
      Then 診断イベントが出力される
      And 判定に必要な状態は Rust 側が所有する

  Rule: workflow step session でも checkpoint に到達して回答できる

    Scenario Outline: workflow step session の pending checkpoint を開く
      Given workflow step session が回答待ちの pending human checkpoint を持つ
      When ユーザーが <導線> から開く
      Then permission dialog が表示される
      And ユーザーは回答できる

      Examples:
        | 導線     |
        | 一覧     |
        | detail   |
```

## 受け入れ観点（振る舞いとの対応）

- 通常表示 / fallback 表示 / 二重表示防止 → R3, R4
- reload・tab 移動・後から open での復元と回答 → R1, R2, R5
- emit 抑止中でも fallback だけで回答 → R10
- finalize が cancelled のまま回帰しない → R6
- backend 所有・frontend は表示のみ → R7
- delta 破棄の warn / 回答待ち停滞の診断イベント → R8, R9
- workflow step session の一覧・detail 導線 → R11

## Open Questions

なし。
