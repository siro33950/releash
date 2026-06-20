# Behavior

terminal 化した workflow run の常駐メモリ解放（#1196）について、外部から観測可能な振る舞いを Gherkin で定義する。

本書は「振る舞いの不変性」を中心に据える。解放そのものは内部構造の変更であり、エンドユーザー・フロント・バックエンドから観測できるのは「履歴表示・状態問い合わせ・active run 進行が変わらないこと」と「run を重ねても常駐メモリが比例して積み上がらないこと」である。`executions` map・Run Store・Event Log・再構築経路等の内部実装名は、観測可能な振る舞いではないため本書の Scenario には持ち込まない。

## 用語

- **terminal run**: status が Completed / Failed / Aborted のいずれかになった workflow run。
- **active run**: status が Running / WaitingApproval の workflow run。
- **履歴問い合わせ**: 完了済み run の状態・ログ・step 出力・step 詳細・run サマリ・worktree 解決といった、run を参照する読み取り操作の総称。

## Feature: terminal run の常駐メモリ解放と振る舞いの不変

  workflow run が terminal 化したとき、その run 本体は常駐メモリから解放される。
  解放後も、エンドユーザー・フロント・バックエンドから観測できる振る舞い
  （履歴問い合わせ結果・active run の進行・worktree 起点の active run 検索）は一切変化しない。

  Background:
    Given workflow runtime が起動している
    And run の状態・履歴は永続化されている

  ## Rule: terminal 化した run の本体は常駐し続けない

    Scenario: 単一 run を完了させても本体が常駐し続けない
      Given 1 つの workflow run を実行している
      When その run が terminal 化する
      Then その run 本体の常駐メモリは解放される

    Scenario Outline: いずれの terminal status でも本体が解放される
      Given 1 つの workflow run を実行している
      When その run が "<status>" になる
      Then その run 本体の常駐メモリは解放される

      Examples:
        | status         |
        | Completed      |
        | Failed         |
        | Aborted        |

    Scenario: 複数 run を完了させても常駐メモリが run 数に比例して積み上がらない
      Given 同種の workflow run を複数回 実行して順次 terminal 化させる
      When run の完了回数を増やしていく
      Then terminal 化済み run に由来する常駐メモリは run 数に比例して増加しない

    Scenario: step 出力が大きい run を完了させても常駐メモリが出力量に比例して積み上がらない
      Given step 出力量の大きい workflow run を実行している
      When その run が terminal 化する
      Then その run の step 出力量に比例した常駐メモリは残らない

  ## Rule: terminal run の履歴問い合わせは解放の前後で同一の結果を返す

    Scenario Outline: 各履歴問い合わせが解放後も従来どおりの結果を返す
      Given 完了済みの workflow run がある
      When その run に対して "<query>" を要求する
      Then 解放前と同一の結果が返る

      Examples:
        | query        |
        | run サマリ取得  |
        | run 状態取得    |
        | run ログ取得    |
        | step 出力取得   |
        | step 詳細取得   |
        | run 一覧取得    |

    Scenario: 完了直後に履歴表示を要求しても従来どおり表示できる
      Given 1 つの workflow run を実行している
      When その run が terminal 化した直後に履歴表示を要求する
      Then その run の履歴が従来どおり表示される
      And 体感的な表示応答性に劣化が生じない

    Scenario: 同一 terminal run に対する履歴問い合わせを繰り返しても結果が一貫する
      Given 完了済みの workflow run がある
      When その run に対して履歴問い合わせを複数回 行う
      Then いずれの問い合わせも同一の結果を返す

  ## Rule: active run の振る舞いは本修正の影響を受けない

    Scenario: active run の進行と状態遷移が変化しない
      Given workflow run が実行中である
      When run が step を進め状態が遷移する
      Then 進行・状態遷移・状態通知（broadcast）は従来どおり行われる
      And active run 本体は解放されない

    Scenario: worktree 起点の active run 検索が変化しない
      Given 特定の worktree に紐づく active run がある
      When その worktree を起点に active run を検索する
      Then 従来どおり該当の active run が見つかる

    Scenario: terminal 化した run は active run 検索の対象から外れる
      Given ある worktree に紐づいていた run が terminal 化している
      When その worktree を起点に active run を検索する
      Then その run は active run としては返らない
      And その run の履歴は履歴問い合わせから参照できる

  ## Rule: 解放と再構築が競合しても状態の不整合や取りこぼしが起きない

    Scenario: terminal 化直後の状態問い合わせで不整合が生じない
      Given 1 つの workflow run が terminal 化する
      When terminal 化とほぼ同時にその run の状態問い合わせが行われる
      Then 状態の不整合やエラーなく、その run の terminal 状態が返る

    Scenario: 複数 run の並列実行・完了で run の取りこぼしが起きない
      Given 複数の workflow run を並列に実行している
      When それらが相次いで terminal 化する
      Then いずれの run も履歴問い合わせから漏れなく参照できる
      And active run の進行は他 run の terminal 化の影響を受けない

    Scenario: terminal 化した run を後から再開・参照しても整合する
      Given terminal 化済みの workflow run がある
      When その run を後から参照・再開しようとする
      Then 永続化された状態に基づき整合した結果が得られる

  ## Rule: 既存の品質ゲートを通過する

    Scenario: 既存のテストと lint が green である
      Given 本修正を適用したコードベースがある
      When cargo test / pnpm test / cargo clippy -D warnings / pnpm lint を実行する
      Then すべて成功する

## 仮定

- [仮定] terminal run の解放は「即時解放」（terminal 化と同時に本体を常駐メモリから外し、必要時に永続化済み状態から再構築）で行う。「直近 N 件保持」「遅延解放」等の保持戦略は採用しない（requirements の合意済み仮定に準拠）。
- [仮定] terminal run の履歴問い合わせは、全 terminal status（Completed / Failed / Aborted）・全問い合わせ経路（run サマリ・状態・ログ・step 出力・step 詳細・一覧）について、解放後も永続化済み状態から従来と同一の結果を供給できる。この網羅性は design / 実装フェーズで裏取りする。
- [仮定] terminal 化直後に履歴表示を要求した場合の再構築コストは、ユーザーの体感的な表示応答性を劣化させない範囲に収まる。劣化が観測された場合は design フェーズで保持戦略を再検討する。
- [仮定] 「常駐メモリが run 数・step 出力量に比例して積み上がらない」ことの確認手段（実測の取り方）は本書では振る舞いとしてのみ規定し、具体的な計測方法は design / 実装フェーズで定める。

## Open Questions

なし（解放方針はユーザー合意により「即時解放」で確定）。
