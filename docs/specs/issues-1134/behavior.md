# Behavior

対象 Issue: #1134「[impl] cli/mod.rs split」

本書は #1134 の受け入れ基準を、実装詳細を含まない観測可能な振る舞いとして定義する。
本 ISSUE はリファクタリング（構造変更）であり、`src-tauri/src/cli/mod.rs` を subcommand 単位の private module へ分割する。
外部から観測される CLI behavior（コマンド体系・引数・出力・終了コード）と、公開 Rust 境界経由で観測される `render_long_help()` の出力は不変であることが中核の要求である。
したがって振る舞いは「構造変更の前後で観測結果が変わらないこと」と「品質ゲートが通ること」を中心に記述する。

## 用語と観測点に関する仮定

- **AS1**: ここでの「振る舞い」は外部観測点で観測されるものを指す。具体的には次に限る。
  - `releash` CLI を起動して subcommand を実行したときの、人間向け出力 / JSON 出力 / 標準エラー出力 / プロセス終了コード。
  - 公開 API `cli::render_long_help()` が返す long help 文字列。
  - 品質ゲート（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）の成否。
  どの subcommand 実装がどの private module へ移ったか、`mod.rs` の内部構造がどう変わったか、internal な subcommand enum / helper の配置といった実装経路は観測点に含めない（それらは requirements / design が扱う）。
- **AS2**: 「構造変更前」は本 ISSUE 着手前の `main` 相当、「構造変更後」は本 ISSUE 完了時点を指す。
- **AS3**: 品質ゲートは CI と同一コマンド（`src-tauri/` で `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）で検証する。
- **AS4**: 「公開 entrypoint」は crate 外（lib / 他 crate / bin）から到達可能な CLI 境界を指し、`cli::run()` と `cli::render_long_help()` の 2 つのみを指す。clap AST・internal subcommand enum（`TopCommand` / `WorkflowSubcommand` / `OutputSubcommand` / `ReviewSubcommand`）は crate 外から到達できないことを観測点とする。

---

Feature: cli/mod.rs split における CLI 振る舞いの不変性

  肥大化した `src-tauri/src/cli/mod.rs` を subcommand 単位の private module へ分割し、
  公開 Rust 境界を `cli::run()` / `cli::render_long_help()` に限定する構造変更を行う。
  この構造変更は、外部から観測される CLI コマンドの振る舞いと long help 出力を変えない。

  Background:
    Given clean architecture へ段階移行中のバックエンドである
    And `src-tauri/src/cli/mod.rs` に CLI の全責務（引数 parse / 出力整形 / 各コマンド実装 / 共有 helper / CLI-only tests）が同居している
    And 本 ISSUE は CLI の user-visible behavior を変えない純粋な構造変更である

  Rule: 構造変更は外部観測可能な CLI コマンドの振る舞いを変えない

    Scenario Outline: 各 subcommand の実行結果が構造変更前後で一致する
      Given 構造変更前の CLI と構造変更後の CLI がある
      When 同一の引数で <command> を実行する
      Then 標準出力・標準エラー出力・終了コードは構造変更前後で一致する

      Examples:
        | command          |
        | workflow         |
        | review           |

    Scenario: コマンド体系（コマンド名・引数）が構造変更前後で一致する
      Given 構造変更前後の CLI がある
      When 利用可能な subcommand とその引数を列挙する
      Then コマンド名・引数・サブコマンド階層は構造変更前後で一致する

    Scenario Outline: 出力形式が構造変更前後で一致する
      Given 構造変更前後の CLI がある
      When ある subcommand を <format> 出力指定で実行する
      Then <format> の出力内容は構造変更前後で一致する

      Examples:
        | format     |
        | 人間向け    |
        | JSON       |

    Scenario: 終了コードが構造変更前後で一致する
      Given 構造変更前後の CLI がある
      When 正常系・異常系それぞれの引数で subcommand を実行する
      Then 返るプロセス終了コードは構造変更前後で一致する

  Rule: 公開 Rust 境界は run() と render_long_help() に限定される

    Scenario: crate 外へ公開する CLI entrypoint は run() に限定される
      Given 構造変更後の CLI module がある
      When crate 外から到達可能な CLI entrypoint を列挙する
      Then `cli::run()` と既存公開の `cli::render_long_help()` のみが到達可能である

    Scenario: internal な clap AST / subcommand enum は crate 外へ公開されない
      Given 構造変更後の CLI module がある
      When crate 外から到達可能な型を列挙する
      Then clap AST と internal subcommand enum（`TopCommand` / `WorkflowSubcommand` / `OutputSubcommand` / `ReviewSubcommand`）は到達可能な型に含まれない

    Scenario: render_long_help() の出力が構造変更前後で一致する
      Given 引数定義 struct/enum を private module へ移送した構造変更後の CLI がある
      When `cli::render_long_help()` を呼び出す
      Then 返る long help 文字列は構造変更前と一致する
      And その long help は Agent backend 起動時の system_prompt へ従来どおり append できる

  Rule: CLI module は layer 境界を越えない

    Scenario Outline: CLI module は禁止 module を import しない
      Given 構造変更後の CLI module 群がある
      When CLI module の import を確認する
      Then <forbidden> への直接 import は存在しない

      Examples:
        | forbidden              |
        | crate::review_comments |
        | crate::git_host        |
        | crate::notion          |
        | crate::protocol        |
        | crate::ws_server       |
        | crate::ws_bridge       |

    Scenario: CLI module は引数 parse と出力整形に責務を限定する
      Given 構造変更後の CLI module 群がある
      When ある subcommand を実行する
      Then CLI module は引数 parse と terminal output formatting を行い、それ以外の処理は usecase / controller-safe boundary の呼び出しに委ねる
      And domain behavior・persistence・workflow execution・review/comment state transition は CLI module 内で実装されない

  Rule: 品質ゲートが通る

    Scenario: format / lint / test がすべて成功する
      Given 構造変更後のリポジトリ状態がある
      When `cargo fmt --check` を実行する
      And `cargo clippy -- -D warnings` を実行する
      And `cargo test` を実行する
      Then いずれのコマンドも成功で終了する

    Scenario: 既存 CLI-only tests が緑のままである
      Given 構造変更前に成功していた CLI-only tests 群がある
      When tests を分割後の各 module へ再配置して実行する
      Then すべてのテストが成功する
      And 振る舞いを変えるためのテスト期待値の書き換えや検証内容の弱体化は行われていない

---

## 受け入れ基準との対応

- **AC1**（`mod.rs` が module wiring と `run()` 中心に縮小）は内部構造の性質であり、外部観測点を持たない。本書では「Rule: 公開 Rust 境界は run() と render_long_help() に限定される」と「Rule: 構造変更は外部観測可能な CLI コマンドの振る舞いを変えない」を通じて、縮小後も観測結果が変わらないことのみを振る舞いとして規定する。分割粒度・配置先の妥当性は requirements（R2）および design が扱う。
- **AC2**（`cli::run()` 以外を公開 entrypoint にしない／`render_long_help()` 維持）は「Rule: 公開 Rust 境界は run() と render_long_help() に限定される」で規定する。
- **AC3**（禁止 module を import しない）は「Rule: CLI module は layer 境界を越えない」で規定する。
- **AC4**（既存 CLI behavior と tests の維持）は「Rule: 構造変更は外部観測可能な CLI コマンドの振る舞いを変えない」と「Rule: 品質ゲートが通る」の既存テスト維持 Scenario で規定する。
- **AC5**（品質ゲート）は「Rule: 品質ゲートが通る」で規定する。

## 仮定

- **AS1 / AS2 / AS3 / AS4**: 上記「用語と観測点に関する仮定」のとおり。
- **AS5**: 本 ISSUE は behavior 不変が前提のリファクタリングであるため、新規の CLI behavior を追加する Scenario は持たない。検証は既存 CLI-only tests（`cargo test`）と、コマンド出力・終了コードが構造変更前後で変わらないことの確認に依拠する（requirements A7 と整合）。
- **AS6**: 網羅的な新規 CLI テストの追加は本 ISSUE のスコープ外とする（requirements A7 と整合）。
- **AS7**: AC3 が列挙する禁止 module への直接 import は構造変更前の時点で既に存在しない（requirements A3）。本書の「CLI module は layer 境界を越えない」Rule は、分割後もこの状態を維持し regression を作らないことを観測点とする。

## Open Questions

なし。
