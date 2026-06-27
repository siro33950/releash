# Behavior

requirements.md（#1250）に基づき、Workflow runtime の失敗処理の観測可能な振る舞いを Gherkin で定義する。
ここでは「失敗をどう分類し、各分類がどう扱われ、観測者（telemetry / run 結果）から何が見えるか」という外部観測可能なビジネスルールのみを扱う。
具体的な型名・配置・既定値（retry 回数上限・各 timeout 秒数・telemetry の送出形式）は design.md で確定する実装詳細であり、本書では扱わない。

## 用語

- **failure kind**: 失敗の発生源による分類。startup timeout / stale runtime timeout / model refusal / structured output mismatch / validation failure / user abort / infrastructure crash。
- **disposition（扱い）**: failure kind が取りうる扱い。retryable（再試行可）/ partial（部分成功として受容）/ terminal（回復不能で終了）/ user-action-required（人間の操作を要する）。
- **enforcement**: 分類と各ポリシーに従って runtime が実際に retry / timeout 適用 / repair / 伝播を行うこと。

---

```gherkin
Feature: Workflow runtime の失敗分類とポリシー enforcement

  Workflow / Step の失敗を発生源ごとに分類し、各分類の disposition に従って
  retry・timeout 適用・structured output repair・parallel failure 伝播を行う。
  失敗の性質は run 結果および telemetry から観測できる。

  Background:
    Given Workflow run が起動している
    And 失敗は発生源ごとの failure kind に分類される
    And 各 failure kind は retryable / partial / terminal / user-action-required のいずれかの disposition を持つ

  # ------------------------------------------------------------------
  # Rule: 失敗は発生源ごとに区別され、同一の "failed" に潰されない
  # ------------------------------------------------------------------
  Rule: 失敗は failure kind により区別される

    Scenario Outline: 失敗源ごとに failure kind が割り当てられる
      Given step 実行中に "<failure_source>" が発生する
      When runtime が失敗を分類する
      Then failure kind は "<failure_kind>" になる
      And その failure kind の disposition は "<disposition>" として識別できる

      Examples:
        | failure_source                       | failure_kind              | disposition          |
        | Codex app-server の起動遅延          | startup timeout           | retryable            |
        | 重い判断 step の応答停止（stale）    | stale runtime timeout     | retryable            |
        | model refusal / provider policy 拒否 | model refusal             | partial              |
        | structured output の contract 不整合 | structured output mismatch| retryable            |
        | 出力 validation の失敗              | validation failure        | terminal             |
        | user による abort                    | user abort                | user-action-required |
        | インフラ層の crash                   | infrastructure crash      | terminal             |

    Scenario: user abort と runtime failure は混同されない
      Given runtime failure（startup timeout）と user abort が同一 run 内で起こりうる
      When それぞれが分類される
      Then user abort は user-action-required として、runtime failure は retryable / terminal として区別される
      And 両者は同一の "failed" 系として集約されない

  # ------------------------------------------------------------------
  # Rule: retryable な失敗は即 fail せず再試行される（TimeoutPolicy / RetryPolicy）
  # ------------------------------------------------------------------
  Rule: retryable な失敗はポリシー上限まで自動 retry される

    Scenario: Codex app-server の起動遅延は即 node_failed にならず retry される
      Given Codex app-server の起動が startup timeout を超える
      When runtime が失敗を扱う
      Then その失敗は startup timeout として分類される
      And 即 node_failed とせず、RetryPolicy の上限まで起動を再試行する
      And 上限内で起動に成功すれば step は失敗扱いにならない
      And turn 経路の thread ready 待機は旧来の約 5 秒固定ではなく、注入済み startup timeout（未注入時は 30 秒）を適用する

    Scenario: retry 上限を超えても回復しない場合は terminal な失敗になる
      Given startup timeout が RetryPolicy の retry 上限まで再試行されても回復しない
      When 上限に達する
      Then step は失敗として終了する
      And 観測者からは failure kind が startup timeout、retry が上限まで行われたことが分かる

    Scenario: 重い判断 step は適用された stale timeout 内であれば失敗扱いにならない
      Given 重い判断 step が長時間処理を継続している
      And その step に node kind / workflow template に応じた stale timeout 値が適用される
      When 処理が適用 timeout 値を超えずに進行する
      Then step は stale runtime timeout として失敗扱いにならない

    Scenario: stale timeout を超えた応答停止は stale runtime timeout として扱われる
      Given step が適用された stale timeout 値を超えて応答を停止する
      When runtime が失敗を扱う
      Then その失敗は stale runtime timeout として分類される
      And RetryPolicy の定めに従って retry または terminal 終了する

  # ------------------------------------------------------------------
  # Rule: structured output の軽微な崩れは repair / reroute される
  # ------------------------------------------------------------------
  Rule: structured output mismatch は repair 上限まで修復が試みられる

    Scenario: 軽微な structured output 崩れは即 fail せず repair される
      Given step の structured output が contract に不整合
      When runtime が StructuredOutputRepairPolicy に従う
      Then 即 fail とせず、repair / reroute が試行される
      And repair 上限内で contract を満たせば step は失敗扱いにならない

    Scenario: repair 上限を超えても整合しない場合は失敗になる
      Given structured output mismatch が repair 上限まで試行されても整合しない
      When 上限を超える
      Then step は失敗として終了する
      And 観測者からは failure kind が structured output mismatch であることが分かる

  # ------------------------------------------------------------------
  # Rule: parallel node の単一子失敗は ParallelFailurePolicy に従って伝播する
  # ------------------------------------------------------------------
  Rule: parallel 子失敗は全体 failed か aggregate 委譲かが決定され、その通り伝播する

    Scenario: review child の model refusal は workflow 全体を巻き込まない
      Given parallel node の 1 つの review child が model refusal で失敗する
      And ParallelFailurePolicy が単一子失敗を aggregate へ委譲すると決定する
      When 失敗が伝播する
      Then workflow 全体は failed にならない
      And 他の child の結果を含めて aggregate へ集約される
      And refusal した child は partial（受容された失敗）として識別できる

    Scenario: 全体 failed と決定された子失敗は workflow を failed にする
      Given parallel node の子失敗について ParallelFailurePolicy が全体 failed と決定する
      When 子が失敗する
      Then workflow 全体が failed になる

  # ------------------------------------------------------------------
  # Rule: 失敗の性質が telemetry から観測できる
  # ------------------------------------------------------------------
  Rule: 失敗発生時に failure kind / retry count / timeout kind が telemetry へ渡せる

    Scenario: 失敗時に分類情報が telemetry へ渡される
      Given step が失敗する
      When 失敗が telemetry へ送出される
      Then failure kind が含まれる
      And retry が行われていれば retry count が含まれる
      And timeout 起因であれば timeout kind が含まれる

    Scenario: telemetry 基盤が未整備でも分類情報を渡す構造が存在する
      Given telemetry 計装基盤（#1209）が未実装である
      When 失敗が発生する
      Then failure kind / retry count / timeout kind を telemetry へ渡せる構造は用意されている
```

---

## 仮定

- **B1（disposition の既定割り当て）**: 上記 Examples の failure kind → disposition の対応は requirements.md「改善する failure mode」4 ケースと非スコープ記述から導いた既定値とする。各 failure kind は単一 disposition に固定されるのではなく「取りうる disposition」を識別できればよく、最終的な扱いは適用ポリシー（RetryPolicy 等）の決定に従う。具体的な対応の確定は design.md で行う。
- **B2（retry / repair 上限超過後の終了）**: retryable / structured output mismatch が各ポリシーの上限を超えても回復しない場合は terminal な失敗として run / step を終了する。上限値（retry 回数・repair 回数）は design.md で確定する。
- **B3（partial の観測単位）**: parallel child の partial 受容は「child 単位で失敗を受容しつつ workflow を継続する」ことを指す。aggregate がその後どう reduce するか（NEEDS_FIX / LGTM 等）の判定ロジックは本書の対象外とし、既存挙動を踏襲する。
- **B4（既存挙動からの後退回避）**: enforcement により observable behavior が変わる箇所には migration / behavior note を残す（requirements A4）。各ポリシーの既定値は現状挙動からの後退を避ける方向で design.md にて確定する。
- **B5（telemetry 送出形式）**: failure kind / retry count / timeout kind を span status / counter / attribute のどれで送るかは実装詳細であり design.md で確定する。本書は「観測者から取得できる」ことのみを規定する。

## Open Questions

なし。
