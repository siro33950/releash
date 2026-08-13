# Context

- 要求の正本: [Issue #1635](https://github.com/siro33950/releash/issues/1635)「refactor(workflow): CLIとYAMLから旧仕様拒絶ガードを削除する」（OPEN）。
- 補助資料（現状確認と影響範囲の特定に使用）:
  - `docs/workflow-yaml-syntax.md` — 受理する WorkflowDefinition YAML の正本。
  - `docs/workflow-engine-evolution-plan.md` — engine 全体方針の一次 owner 文書。品質ゲートに「valid / invalid fixture の Diagnostic code と stage を固定する」を掲げている。
  - CLI 経路: `src-tauri/src/cli/mod.rs`、`src-tauri/src/cli/cli_test.rs`、`src-tauri/src/cli/output_test.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host/prompt_rendering.rs`、`src-tauri/src/adaptor/gateway/workflow/facet.rs`。
  - Workflow YAML 経路: `src-tauri/src/domain/workflow/value_objects/definition.rs`、`src-tauri/src/domain/workflow/services/contract_schema.rs`、`src-tauri/src/adaptor/gateway/workflow/{schema_contract_tests.rs,storage.rs,definition_repository.rs,diagnostics.rs,builtin.rs}`、`src-tauri/src/adaptor/gateway/workflow/fixtures/invalid/`。
- 背景: CLI と Workflow YAML の仕様変更時に、現在の入力契約を検証するテストへ置き換えず、削除した command・flag・field・語彙を名指しで拒否する blacklist、fixture、source scan を追加してきた。その結果、現行仕様と無関係な過去仕様がテストと validation に蓄積している。
- 要求元が確定した方針（後続の Behavior・Design が従う）:
  - 現在サポートする入力と、その意味上の制約だけを仕様・テストにする。
  - 未知の入力を拒否する契約は現行仕様として維持する。CLI の未知 option は parse error、Workflow YAML の未知 field / keyword は Diagnostic とする。
  - 過去の command・flag・field・語彙を blacklist として保持しない。旧構文であることを理由とする専用の Diagnostic code を持たない。
  - 現在仕様の必須値、型、相互排他、参照整合性、状態遷移の検証は維持する。
  - 未知入力の拒否を確認するテストでは、過去の具体名ではなく `future-option` / `future_field` のような一般名を使う。
- 制約: 本リポジトリの既定の品質ゲート（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm lint` / `pnpm test` / `pnpm build`）を通す。

# Outcome

- 対象者: Releash 本体を保守する開発者、および Workflow YAML を書く workflow author。
- 現在の問題: 削除済みの command・flag・field・語彙の blacklist が CLI / Workflow YAML 経路のテストと validation に蓄積し、現行仕様のテストと混在している。どのテストが現在の契約を守っているのかが読み取れず、仕様変更のたびに blacklist が増える。旧構文専用の Diagnostic code が、拒否理由の分類を過去仕様に依存させている。
- 変更後に実現する状態: CLI と Workflow YAML は、現在サポートする入力と、その意味上の制約だけを検証する。未知入力の拒否は現行どおり維持され、その理由が過去仕様かどうかで分岐しない。過去の構文を「拒否されること」で固定するテスト・fixture・source scan は CLI / Workflow YAML 経路に存在しない。

# Current Behavior

commit `55410de53a86b814ffee1ec05e5c083044489687`（branch `feat/issues/1635`）の worktree で、以下をコード調査により確認した。挙動の記述は、現在 CI で通っている既存テストの assertion と、それを成立させている実装箇所の両方に基づく。

## 維持する挙動

- CLI 入口は clap derive の既定設定で、未知 option を parse error にする（`src-tauri/src/cli/mod.rs:24-29`、`:103`）。未知 subcommand、必須引数の欠落、既知引数の型不正も parse error になる。
- Workflow YAML DTO 10 箇所に `#[serde(deny_unknown_fields)]` が付いている（`src-tauri/src/domain/workflow/value_objects/definition.rs` の `WorkflowDefinition`:15、`CommandSpec`:98、`FacetRefs`:112、`SessionSpec`:134、`FanoutSpec`:179、`RawNodeDefinition`:276、`WhenRule`:448、`SwitchRule`:455、`LoopGuardRule`:462、`RawRule`:471）。deserialize 段階で未知 field が拒否される。
- schema map（`schemas:`）は、現在使う keyword 以外を拒否する（`src-tauri/src/domain/workflow/services/contract_schema.rs:49`、`:77`、`:92`、`:337`）。
- raw YAML の shape 段階で、root / node / session / session facets / fanout / rule 要素の許可 field を列挙し、列挙外の key を Error Diagnostic にする（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:528-574` の `check_allowed_fields`）。
- 現在仕様の必須値、型、相互排他、参照整合性、状態遷移の検証。

## 除去する対象

- 旧構文専用の Diagnostic code と、その分類のための旧 field 一覧。`check_allowed_fields` は、許可 field 外の key が旧 field 一覧に含まれる場合は `WFS005`（`field '<key>' belongs to the old workflow syntax and is no longer accepted`）、含まれない場合は `WFS002`（`unknown workflow field '<key>' is not allowed here`）を出し分ける。旧 field 一覧は次のとおり。
  - root（`:257-266`）: `steps` / `variables` / `workflow_variables` / `tasks`。
  - node（`:308-338`）: `type` / `mode` / `facets` / `prompt` / `inline_prompt` / `output_contract` / `input_contracts` / `pass_output_from` / `pass_previous_response` / `variables` / `workflow_variables` / `cycle_guard` / `resets_cycle_for` / `collect` / `approval` / `bash` / `parallel` / `tasks`。
  - session block（`:402-411`）: `mode` / `prompt` / `inline_prompt`。
  - fanout block（`:443-460`）: `parallel_children` / `aggregate` / `all_match` / `any_match` / `failure_policy` / `on_failure` / `fail_fast`。
  - fanout aggregate（`:465-475`）: `all_match` / `any_match` / `then` / `else`。許可 field が空で、旧 field の名指しだけを目的とする。
  - rule 要素（`:483-500`）: `match` / `regex` / `expression` / `condition` / `cycle_guard` / `resets_cycle_for` / `reject` / `rerun`。
- deserialize エラーの分類が、メッセージ本文で旧構文を判定する（`:595-628`：`old workflow syntax` を含むなら `WFS005`）。
- 削除済み CLI surface を名指しで拒否する assertion。
  - `src-tauri/src/cli/cli_test.rs:19-32`: long help に `start` / `executions` / `logs` / `approve` / `abort` / `stop` / `resume` / `output validate` / `list` が含まれないことを列挙して検査する。
  - `src-tauri/src/cli/cli_test.rs:36-73`: 上記 9 種の argv が parse error になることを検査する。
  - `src-tauri/src/cli/cli_test.rs:111-140`: `docs/workflow-engine-evolution-plan.md` の本文に、削除済み CLI surface 9 種の文字列が含まれないことを検査する。
  - `src-tauri/src/cli/cli_test.rs:171-184`: `mod.rs` / `workflow.rs` / `output.rs` / `api_client.rs` / `file_direct.rs` の source 文字列に旧語彙 `workflow_pending` と `CliMutationRequested` が現れないことを検査する。
  - `src-tauri/src/cli/output_test.rs:85-98`: 削除済み `workflow output validate` が parse error になることを検査する。
  - `src-tauri/src/cli/output_test.rs:137-148`: 旧 Submit 構文（positional `<execution-id>` + `--node`）が parse error になることを検査する。
- prompt に削除済み flag が現れないことを名指しする negative assertion（`src-tauri/src/adaptor/gateway/workflow/workflow_host/prompt_rendering.rs:468`、`:504-508`、`:527-528`、`:552-553`）。
- 削除済み CLI / event 語彙の blacklist（`src-tauri/src/adaptor/gateway/workflow/facet.rs:324-343`、9 語彙）。唯一の呼び出し元は、blacklist 自身が機能することを検査する自己テスト（`:358-372`）である。
- 過去の構文が拒否されることを固定するテスト。
  - `src-tauri/src/adaptor/gateway/workflow/schema_contract_tests.rs`: 旧 `model:` / `permission:`（`:10-40`）、旧 rule `match`（`:373`）、旧 rule `reject` / `rerun`（`:391`）、node 直下の `cycle_guard` / `resets_cycle_for`（`:455`）、旧 `type:`（`:504`）、旧 `output_contract`（`:519`）、旧 `input_contracts`（`:536`）、retired `additionalProperties`（`:613`）、flat facets（`:644`）、`inline_prompt`（`:661`）、kind block 内外の余剰 field（`:678` / `:694` / `:706` / `:722`）、`parallel_children`（`:739`）。
  - `src-tauri/src/adaptor/gateway/workflow/storage.rs`: `load_workflow_rejects_legacy_steps_yaml`（`:829`）、`load_workflow_rejects_variables_section`（`:1019`）、`load_workflow_rejects_legacy_pass_fields`（`:1045`）。
  - `src-tauri/src/adaptor/gateway/workflow/definition_repository.rs`: `invalid_legacy_source`（`:272-283`、`type: agent` / `instruction:` を持つ旧 YAML）とそれを使う保存失敗テスト（`:327-345`）。
  - `src-tauri/src/adaptor/gateway/workflow/builtin.rs:543-550`: builtin workflow `03_full-review` の source に `model:` / `permission:` が現れないことを検査する。
- `src-tauri/src/adaptor/gateway/workflow/fixtures/invalid/` の `WFS005_legacy-*` 23 件と、それを必須 fixture として manifest 化する `legacy_cleanup_regression_fixture_manifest_is_complete`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:1761-1809`）。

# Scope / Non-goals

## Scope

- 過去の CLI command・flag・YAML field・event 語彙を名指しで拒否する blacklist、source scan、negative assertion、fixture、および fixture を必須化する manifest の削除。
- 旧構文専用 Diagnostic code `WFS005` と、その分類に必要な旧 field 一覧および deserialize メッセージ判定の削除。未知 field は現行どおり `WFS002` として拒否する。
- 許可 field が空で旧 field の名指しだけを目的とする shape 検査（fanout aggregate）の削除。
- 未知入力の拒否を確認するテストを、過去の具体名ではない一般名で書くこと。

## Non-goals

- CLI の未知 option、未知 subcommand、必須引数の欠落、既知引数の型不正の受理。これらは従来どおり拒否する。
- Workflow YAML の未知 field / keyword の受理。これも従来どおり拒否する。
- 永続データの version migration、event replay、archive recovery など、保存済みデータを安全に読むための互換処理。
- security boundary、認証、path traversal、secret masking の拒否テスト。
- 現在仕様として定義されている必須値、型、相互排他、参照整合性、状態遷移の検証（維持対象であり、緩和対象ではない）。
- 削除済みの CLI command / YAML field を互換 alias として復活させること、および正規化 layer の導入。
- 過去の milestone spec 文書（`docs/specs/milestone-82/design.md` が旧 field と WFS005 / WFS002 の対応を記述している）の書き換え。
- `docs/workflow-yaml-syntax.md` と `docs/workflow-engine-evolution-plan.md` の書き換え。両文書が定義する unknown field rejection は現行仕様として維持される。
- frontend の変更。`src/hooks/useAutomation.test.ts` は `WFS005` を Diagnostic 表示テストの stub 文字列として使うが、backend が返す code の集合には依存していない。

# Requirements

- R-001: CLI 経路と Workflow YAML 経路に、過去の command・flag・field・語彙を名指しで拒否する blacklist、source scan、negative assertion、fixture が存在しない。過去の具体的な構文を「拒否されること」で固定するテストが存在しない。
- R-002: 旧構文であることを理由とする Diagnostic code と、その分類のための旧 field 一覧が存在しない。未知 field / keyword はその理由によらず同一の code で拒否される。
- R-003: 互換性要件 — CLI の未知 option、未知 subcommand、必須引数の欠落、既知引数の型不正は、変更前と同じくエラーになる。
- R-004: 互換性要件 — Workflow YAML の top-level、node、kind block、session facets、rule 要素、schema map の未知 field / keyword は、変更前と同じく拒否される。
- R-005: 互換性要件 — 現在仕様として定義されている必須値、型、相互排他、参照整合性、状態遷移の検証は、変更前と同じ入力に対して同じ判定を返す。
- R-006: 未知 option / field が拒否されることを確認するテストが、過去に存在した具体名ではなく、その時点で仕様に存在しない一般名を使う。

# Assumptions / Open Questions

なし。
