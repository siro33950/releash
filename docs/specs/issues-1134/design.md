# Design

対象 Issue: #1134「[impl] cli/mod.rs split」

本書は `requirements.md` / `behavior.md` を前提に、`src-tauri/src/cli/mod.rs`（約 3648 行）を
subcommand / 責務単位の private module へ分割する実装方針を定める。
本 ISSUE は user-visible behavior を変えない純粋な構造変更（リファクタリング）であり、
新規 behavior は持たない。

## 概要

- 単一ファイル `cli/mod.rs` に同居している「引数定義 / dispatch / 各コマンド実装 / 共有 helper /
  CLI-only tests」を、subcommand 単位の private module へ移送する。
- 公開 Rust 境界は `cli::run()` と `cli::render_long_help()` の 2 つのみに維持する。
- clap の arg 定義型（`Cli` / `TopCommand` / `WorkflowSubcommand` / `OutputSubcommand` /
  `ReviewSubcommand`）は private のまま、配置だけを各 subcommand module へ移す。
- `mod.rs` は module 宣言 + `Cli`/`TopCommand` の root 定義 + `run()` の dispatch +
  `render_long_help()` 中心に縮小する。
- 既存 import が既に `adaptor` / `usecase` / `domain` layer 経由であること（requirements A3）を
  分割後も維持し、禁止 module（`crate::review_comments` 等）への直接 import を作らない。

## 変更対象

- `src-tauri/src/cli/mod.rs`（分割元）
- 新規 private module ファイル（下記「アーキテクチャと責務分割」参照）
- 同ファイル内の CLI-only tests（`#[cfg(test)] mod tests`、73 個の test 関数）を各 module へ再配置

`cli` を import する呼び出し元（`lib.rs` / bin entry / Agent backend 起動経路）は、公開 API が
不変のため変更しない。

## アーキテクチャと責務分割

### module 構成（決定）

`cli/` 配下を以下のフラットな private module 構成に分割する。

```text
src-tauri/src/cli/
├── mod.rs        # module wiring + Cli/TopCommand root + run() dispatch + render_long_help()
├── common.rs     # CLI 横断の共有: CliError、data dir 解決、共有 validation/format helper
├── workflow.rs   # WorkflowSubcommand + list/runs/status/logs/approve/reject/abort + file-direct 読取
├── workflow_io.rs # workflow/output 共有の file-direct run/log 読取 + pending enqueue 境界
├── output.rs     # OutputSubcommand + output submit/validate/get + contract validation helper
└── review.rs     # ReviewSubcommand + cmd_review + review thread/comment/history の表示・検証 helper
```

各 module は private（`mod foo;`、`pub` を付けない）とし、crate 外へ漏らさない。
module 間で必要な型・関数は `pub(crate)` ではなく **module-private + `use` による crate 内可視**で
足りる範囲に留める。具体的には `mod.rs` が各 subcommand の handler 関数と arg 型を `use` できれば
よいので、それらは `pub(super)`（= `cli` module ツリー内可視）とする。crate 全体へ公開しない。

### module ごとの責務

#### `mod.rs`（縮小後）

- clap root 型 `Cli` と `TopCommand` の定義を保持する。
  - `TopCommand::Workflow { command: workflow::WorkflowSubcommand }`、
    `TopCommand::Review { command: review::ReviewSubcommand }` のように、各 subcommand enum を
    対応 module から参照する。
- `pub fn render_long_help() -> &'static str`: 現状どおり `Cli::command().render_long_help()` を
  `OnceLock` キャッシュで返す。clap derive のツリーは型の配置に依存せず辿られるため、arg enum を
  module へ移しても出力は不変。
- `pub fn run() -> i32`: parse → `resolve_existing_data_dir()` → `TopCommand` の match dispatch。
  各 arm は対応 module の handler 関数（`workflow::cmd_list` 等）を呼ぶだけに留める。
- `cli_result_exit_code()` による `CliError` → 終了コード変換は、終了コード規約の単一所有点として
  `mod.rs`（または `common.rs`）に置く。本設計では `CliError` 定義と一体で `common.rs` に置き、
  `mod.rs` は `common::cli_result_exit_code` を呼ぶ。

#### `common.rs`

CLI 横断で複数 module が使う最小限の共有要素のみを置く。

- `CliError`（enum: `NotFound` / `InvalidInput` / `Other`）と `From<String>` 実装。
- `cli_result_exit_code(Result<(), CliError>) -> i32`（終了コード規約の単一所有点）。
- data dir 解決: `resolve_data_dir` / `resolve_data_dir_from_env` / `resolve_existing_data_dir` /
  `ensure_existing_data_dir`。
- 全 subcommand で共有する汎用 helper のみ: `truncate`、`validate_run_id`、
  `validate_optional_cli_text_len`（複数コマンドの free-text 長検証）。
- workflow 系のみで使う event 整形（`event_draft_to_cli_log_json` / `format_event_draft` /
  `event_kind_display_name` / `reconstruct_state_view` / workflow list/runs の file-direct 読取群）は **`workflow.rs` に置き、
  `common.rs` には置かない**（単一 module 専用の helper を共有層に上げない）。

#### `workflow.rs`

- `WorkflowSubcommand` enum（`Output` variant は `output::OutputSubcommand` を参照）。
- handler: `cmd_list` / `cmd_runs` / `cmd_status` / `cmd_logs` / `cmd_enqueue_pending`。
- approve/reject/abort 系: `validate_reject_reason`、`approval_input_error_to_cli_error`。
- file-direct 観測: `list_workflows_file_direct` / `running_workflow_names_file_direct` /
  `list_runs_file_direct` / `canonicalize_cli_worktree_filter_path` /
  `reconstruct_state_view`。
- workflow event 整形 helper（上記）。
- これらが import する `adaptor::gateway::workflow::*` / `domain::workflow::*` /
  `usecase::workflow::*` は workflow.rs と下記 `workflow_io.rs` の責務内に閉じる。

#### `workflow_io.rs`

- `workflow.rs` と `output.rs` の両方が使う workflow file-direct I/O 境界のみを置く。
- `CliRequestPayload` / `PendingEnqueueOutput` / `enqueue_pending_command`。
- run summary 参照: `get_run_summary_file_direct`。
- domain event log 読取: `read_domain_log`。
- `workflow.rs` / `output.rs` 間の相互依存を作らないための専用 private module であり、
  CLI 横断の `common.rs` には上げない。

#### `output.rs`

- `OutputSubcommand` enum（`Submit` / `Validate` / `Get`）。
- handler: `cmd_output_submit` / `cmd_output_validate` / `cmd_output_get`。
- helper: `validate_step_argument` / `validate_contract_argument` / `read_submit_input_json` /
  `resolve_step_output_contract_via_log` / `validate_cli_contract_output` /
  `OutputGetView` / `build_output_get_view`。
- 判断: `Output` は clap 上 `workflow output ...` の sub-subcommand だが、contract 検証という
  独立した関心事と相応のコード量（handler + helper + tests）を持つため、`workflow.rs` のネスト
  module ではなく独立 module とする（requirements A5 / 下記「仮定」A-D1）。

#### `review.rs`

- `ReviewSubcommand` enum（`List` / `Get` / `Create` / `Comment` / `Resolve` / `History`）。
- handler: `cmd_review`。
- review actor / worktree 解決: `review_actor` / `review_actor_and_worktree` /
  `review_worktree_from_session`。
- 入力 parse: `parse_review_state` / `parse_optional_author_scope` / `parse_optional_unread`。
- error 変換・表示: `review_error_to_cli_error` / `write_cli_error` / `print_review_thread` /
  `write_review_thread` / `write_review_thread_list` / `write_review_history`。
- import する `domain::comment::*` / `usecase::comment::*` /
  `adaptor::controller::wiring::build_review_comment_usecase` は review.rs に閉じる。

### layer 境界（R3 / behavior「CLI module は layer 境界を越えない」）

- 各 module は引数 parse と terminal output formatting のみを持つ。実処理は usecase /
  controller-safe boundary（`build_review_comment_usecase` 等）、gateway repository、
  domain helper の呼び出しに委ねる。
- 禁止 import（`crate::review_comments` / `crate::git_host` / `crate::notion` /
  `crate::protocol` / `crate::ws_server` / `crate::ws_bridge`）は分割前後で 0 件を維持する。
  - 現状 import は `crate::adaptor::protocol::workflow::WorkflowStateView` を使うが、これは
    `crate::protocol`（禁止対象）ではなく `crate::adaptor::protocol`（adaptor layer 経由）であり、
    AC3 の禁止リストには該当しない。分割後も同じ経路を維持する。

## データモデルまたは型

新規の domain 型・永続化型は導入しない。既存型の **配置のみ**を移送する。

- arg 型: `Cli`（mod.rs）/ `TopCommand`（mod.rs）/ `WorkflowSubcommand`（workflow.rs）/
  `OutputSubcommand`（output.rs）/ `ReviewSubcommand`（review.rs）。可視性は `pub(super)`。
- error 型: `CliError`（common.rs）。
- 表示用 view 型: `CliRequestPayload` / `PendingEnqueueOutput`（workflow_io.rs）、
  `OutputGetView`（output.rs）。各 module 内可視で十分。
- `#[derive(serde::Serialize)]` を持つ型（`CliRequestPayload` / `OutputGetView`）の JSON 表現は
  field 名・形状ともに不変（behavior「出力形式が構造変更前後で一致する」）。

## 処理フロー

`run()` の制御フローは現状を保存する。

1. `Cli::try_parse()`。parse error は clap の `print()` + `exit_code()` で返す（不変）。
2. `WorkflowDefinitionFileRepository::default_workflows_dir()` を解決。
3. `TopCommand` を match し、各 arm 内で `resolve_existing_data_dir()`（`common`）を呼んでから
   対応 module の handler を呼ぶ。data dir 不在は `NotFound`（終了コード 4）として区別する挙動を維持。
4. handler の `Result<(), CliError>` を `common::cli_result_exit_code` で終了コードへ変換。
   - `Ok` → 0、`NotFound` → 4、`InvalidInput` → 2、`Other` → 1。

dispatch の match arm 構造（resolve → handler、Output の sub-match）は現状のロジックをそのまま
移送する。handler 関数のシグネチャは変更しない。

## エラー処理

- `CliError` の 3 variant と終了コードマッピングは不変（behavior「終了コードが構造変更前後で
  一致する」）。
- `module ごとに専用 error type` という Rust 規約はあるが、本 ISSUE は behavior 不変が要件であり、
  終了コード・stderr メッセージを変えてはならない。したがって CLI 共通の `CliError` を分割しての
  module 別 error 型化は **本 ISSUE のスコープ外**とし、`CliError` を `common.rs` の単一 error 型
  として維持する（過度な再設計で behavior regression を作らない方針）。
- 各 module 固有の error 変換（`review_error_to_cli_error` / `approval_input_error_to_cli_error`）は
  それぞれの module に置き、最終的に `CliError` へ畳む。

## テスト方針

- behavior 不変の検証は既存 CLI-only tests（`cargo test`）に依拠する（requirements A7 / behavior AS5）。
  新規の網羅的 CLI テストは追加しない。
- 既存 `mod tests`（73 test）を、テスト対象の関数が属する module へ `#[cfg(test)] mod tests` として
  再配置する。検証内容・期待値は変更しない（behavior「振る舞いを変えるためのテスト期待値の
  書き換えや検証内容の弱体化は行われていない」）。
  - review_* / cmd_review_* → `review.rs`
  - cmd_output_* / cli_workflow_output_* → `output.rs`
  - cmd_list/runs/status/logs/enqueue 系・file_direct 系・worktree filter 系・
    cli_workflow_subcommands_parse_via_clap・cli_does_not_expose_out_of_scope_subcommands →
    `workflow.rs`
  - resolve_data_dir_* / ensure_existing_data_dir_* / run_exit_code_mapping_is_stable /
    render_long_help_* → `common.rs` または `mod.rs`（render_long_help は `mod.rs` 側）。
- テスト用 helper（`write_review_config` / `write_review_session` / `make_run` 等）は、利用する
  test と同じ module の `mod tests` 内へ移す。複数 module から使うものは各 module で必要分のみ複製
  せず、`common.rs` の `#[cfg(test)]` helper として共有する（重複は最小化）。
- 品質ゲート: `src-tauri/` で `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`
  をすべて通す（R5 / AC5）。
- behavior 不変の補助確認として、分割前後で `releash workflow --help` / `releash review --help` /
  代表 subcommand の正常系・異常系出力と終了コードが一致することを手動で突き合わせる
  （behavior「各 subcommand の実行結果が構造変更前後で一致する」）。

## リスクと代替案

- **リスク: `render_long_help()` 出力の差分**。arg 型を module へ移すと、clap が拾う型の定義位置が
  変わる。help 文字列は doc comment と `#[command]`/`#[arg]` 属性から生成されるため、doc comment と
  属性を一字一句そのまま移送すれば出力は不変。`render_long_help_contains_main_subcommands` /
  `render_long_help_is_cached_across_calls` で検証し、必要なら分割前出力をスナップショット比較する。
- **リスク: 可視性過多**。安易に `pub(crate)` を付けると公開境界が曖昧になり R1/AC2 を損なう。
  module 間共有は `pub(super)` 止まりとし、crate 外公開は `run` / `render_long_help` のみに限定する。
  `cli_does_not_expose_out_of_scope_subcommands` 相当の test で arg enum 非公開を担保する。
- **リスク: 循環 / 共有 helper の置き場誤り**。workflow 専用 helper を `common.rs` へ上げると
  layer 整理の意図がぼやける。単一 module 専用 helper はその module に閉じ、`common.rs` は真に横断的な
  ものだけに絞る。
- **代替案: Output を `workflow.rs` 内のネスト module にする**。clap 階層と一致するが、output は
  contract validation という別関心事で行数も多く、独立 module の方が責務境界が読みやすい。本設計は
  独立 module を採用する（A-D1）。
- **代替案: arg 定義を `cli/args.rs` に集約**。help 生成の単一性は得られるが、subcommand 単位の境界
  という R2 の意図に反し、各 subcommand の arg と handler が離れる。本設計は arg を各 subcommand
  module に同居させる。

## 仮定

- **A-D1**: `OutputSubcommand` とその handler/helper/tests は独立 module `cli/output.rs` に置く
  （requirements A5 の「output を独立 module にするか workflow 配下か」を独立 module で確定）。
- **A-D2**: module 間で共有が必要な型・関数の可視性は `pub(super)` を上限とし、`pub` / `pub(crate)`
  を新たに付けない（公開境界は `run` / `render_long_help` のみ）。
- **A-D3**: `CliError` は `common.rs` の単一 CLI error 型として維持し、module 別 error 型への分割は
  行わない（behavior 不変優先。Rust の module 別 error 規約より regression 回避を優先）。
- **A-D4**: 単一 subcommand 専用の helper（workflow event 整形・file-direct 読取等）は所有 module に
  閉じ、`common.rs` には横断的に使われる要素のみを置く。
- **A-D5**: 既存テストの再配置は対象関数と同 module への移送に限り、テスト本体・期待値・helper の
  検証内容は変更しない。`#[cfg(test)]` 専用 helper のみ `common.rs` に共有を許す。
- requirements / behavior の A1〜A7 / AS1〜AS7 を引き継ぐ。

## Open Questions

なし。
