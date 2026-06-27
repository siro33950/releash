# Behavior

対象 Issue: #1133「[impl] root glue / composition root cleanup」

本書は #1133 の受け入れ基準を、実装詳細を含まない観測可能な振る舞いとして定義する。
本 ISSUE はリファクタリング（構造変更）であり、外部から観測される app behavior / UI-visible behavior は不変であることが中核の要求である。
ただし security hardening と通知ライフサイクル整理として、`get_review_text_diff` / `get_review_image_diff` の削除と Closed / Archived の非通知化だけは明示的な観測差分として扱う。
したがって振る舞いは「構造変更の前後で観測結果が変わらないこと」と「品質ゲートが通ること」を中心に記述する。

## 用語と観測点に関する仮定

- **AS1**: ここでの「振る舞い」は外部観測点（起動後の app 動作、menu / tray / native-drop の UI-visible 動作、各 Tauri command の入出力、file watching 由来のイベント通知、notification 発火）で観測されるものを指す。crate root 直下のどの file がどの layer へ移ったか、`lib.rs` の内部構造がどう変わったかといった実装経路は観測点に含めない（それらは design / requirements が扱う）。
- **AS2**: 「構造変更前」は本 ISSUE 着手前の `main` 相当、「構造変更後」は本 ISSUE 完了時点を指す。
- **AS3**: 品質ゲートは CI と同一コマンド（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）で検証する。

---

Feature: root glue / composition root cleanup における振る舞いの不変性

  crate root 直下の platform / app glue を architecture layer へ移送し、
  `lib.rs` を composition root として薄くする構造変更を行う。
  この構造変更は、外部から観測される app / UI / command の振る舞いを変えない。

  Background:
    Given clean architecture へ段階移行中のバックエンドである
    And crate root 直下に platform integration / file watching / permission mode / test helper の glue が残っている
    And 本 ISSUE は app behavior / UI 表示を変えない純粋な構造変更である

  Rule: 構造変更は外部観測可能な app / UI behavior を変えない

    Scenario: app 起動後の振る舞いが構造変更前後で一致する
      Given 構造変更前の app と構造変更後の app がある
      When 同一の操作列で app を起動し操作する
      Then 起動後に観測される app の振る舞いは両者で一致する

    Scenario Outline: platform integration の UI-visible 動作が一致する
      Given 構造変更前後の app がある
      When <surface> を操作する
      Then <surface> の UI-visible な動作は構造変更前後で一致する

      Examples:
        | surface      |
        | menu         |
        | tray         |
        | native drop  |
        | focus 追従    |

    Scenario: 各 Tauri command の入出力が構造変更前後で一致する
      Given 構造変更前後の app がある
      And 同一の引数を持つ任意の登録済み Tauri command がある
      When その command を同一引数で呼び出す
      Then command の成功 / 失敗結果と返却値は構造変更前後で一致する

    Scenario: file watching 由来のイベント通知が構造変更前後で一致する
      Given file watching の対象パスがある
      When 監視対象パスに変更が発生する
      Then 観測される watch イベント通知は構造変更前後で一致する

    Scenario: notification の発火が構造変更前後で一致する
      Given agent status の変化を監視する状態にある
      When agent status が Active / Idle / Error / Done のいずれかへ変化する
      Then 発火する notification の有無と内容は構造変更前後で一致する

  Rule: 完了通知は論理的完了 1 回に収束する

    Scenario: Done 後の Closed / Archived では追加通知しない
      Given agent status の変化を監視する状態にある
      When agent status が Done へ遷移し、その後 Closed、Archived へ遷移する
      Then Done notification は Done 遷移時の 1 回だけ発火する
      And Closed / Archived では notification は発火しない

    Scenario Outline: Closed / Archived 単独遷移は notification を発火しない
      Given agent status の変化を監視する状態にある
      When agent status が <state> へ変化する
      Then notification は発火しない

      Examples:
        | state     |
        | Closed    |
        | Archived  |

  Rule: 起動時タスクは構造変更前と同じ外部効果を生む

    Scenario: 起動時の orphan cleanup が同じ外部効果を生む
      Given 前回実行の orphan が残った状態で app を起動する
      When 起動シーケンスが完了する
      Then orphan cleanup による外部効果は構造変更前後で一致する

    Scenario: 起動時の listener / watcher 登録が同じ観測結果を生む
      Given app を起動する
      When 起動シーケンスが完了する
      Then notification listener と file watcher は構造変更前と同じく稼働し、同じ観測結果を生む

  Rule: 登録済み Tauri command の集合が構造変更前後で一致する

    Scenario: command registration の集約後も呼び出し可能な command 集合が一致する
      Given 構造変更前に登録されていた Tauri command の集合がある
      When 構造変更後の app で登録済み command を列挙する
      Then 呼び出し可能な command の集合は構造変更前後で一致する
      And いずれの command も frontend の `invoke` から従来どおり呼び出せる

    Scenario: legacy review diff command は security hardening として削除される
      Given 構造変更前に `get_review_text_diff` と `get_review_image_diff` が登録されていた
      When 構造変更後の app で登録済み command を列挙する
      Then `get_review_text_diff` と `get_review_image_diff` は呼び出し可能な command 集合に含まれない
      And この 2 command の削除だけが command 集合不変 Rule の明示的な例外である

  Rule: 品質ゲートが通る

    Scenario: format / lint / test がすべて成功する
      Given 構造変更後のリポジトリ状態がある
      When `cargo fmt --check` を実行する
      And `cargo clippy -- -D warnings` を実行する
      And `cargo test` を実行する
      Then いずれのコマンドも成功で終了する

    Scenario: 既存テストが緑のままである
      Given 構造変更前に成功していた既存テスト群がある
      When 構造変更後に同じテスト群を実行する
      Then すべてのテストが成功する
      And 振る舞いを変えるためのテスト期待値の書き換えは行われていない

---

## 受け入れ基準との対応

- AC1（root file の layer 移送）／ AC3（root module 宣言の整理）／ AC4（command registration の集約）は構造的な受け入れ基準であり、外部観測点を持たない内部構造の性質である。本書では「Rule: 登録済み Tauri command の集合が一致する」「Rule: 起動時タスクは同じ外部効果を生む」を通じて、その構造変更が観測結果を変えないことのみを振る舞いとして規定する。構造移送の経路・配置先の妥当性は requirements（R1 / R3 / R4）および design が扱う。
- AC2（`lib.rs` の composition root 化）は内部構造の性質であり、観測点としては「起動時タスクの外部効果不変」に帰着する。
- AC5（品質ゲート）は「Rule: 品質ゲートが通る」で規定する。

## 仮定

- **AS1 / AS2 / AS3**: 上記「用語と観測点に関する仮定」のとおり。
- **AS4**: 本 ISSUE は behavior 不変が前提のリファクタリングであるため、新規の app behavior を追加する Scenario は持たない。検証は既存テストと、構造変更前後で UI-visible behavior が変わらないことの確認に依拠する（requirements A6 と整合）。
- **AS5**: 網羅的な新規 UI テストの追加は本 ISSUE のスコープ外とする（requirements A6 と整合）。

## Open Questions

なし。
