# Context

- 要求の正本: Issue #1850「[01] CI を PR 層と nightly 層に分け、PR の待ち時間を縮める」。
- 背景資料
  - マイルストーン #96「00. Nightly 化と CI 改善」: 検証を 2 層に分ける方針の正本。PR 層（`ci.yml`。PR と main への push）は「merge してよいか」を速く返し、nightly 層（`nightly.yml`。日次と手動）は「リリースしてよいか」を保証して重い検証・構成の網羅・計測系を置く。nightly 層は常に main を対象にし、手動実行は日次を待たずに重い検証を通してそのままリリースするための経路である。
  - Issue #1825: `rust-test` に 4 ステップ、`rust-lint` に 2 ステップを追加した PR。待ち時間が急増した起点。
  - Issue #1851「[02] nightly を基点にリリースする」: 依存に #1850 を持ち、本変更が作る `nightly.yml` と検証のジョブの上に prerelease の作成・stable の作成・古い nightly の削除を載せる。
  - Issue #1766「B11: Xcode 署名・公証・Sparkle による公開」: #1851 の流れの上に Swift.app のビルドと Sparkle の更新配布を載せる。#1766 本文は nightly workflow の起動・スキップ判定・関門・prerelease を #1851 が作ると記すが、正本である #1850 は「トリガーとスキップ判定はこの ISSUE で先に作る」と定めており、本変更が先に作る。
  - `.github/workflows/ci.yml`: 変更対象の現行 PR 層。
- 制約
  - 必須チェックの名前は ruleset `main` が参照する。Rust の検証をまとめたチェックの名前 `rust` は外部との契約である（`ci.yml` の集約ジョブのコメントが理由を記載している）。
  - `workflows/facets/**/*.md` は `adaptor/gateway/workflow/builtin.rs` が `include_str!` でコンパイル時に取り込むため、Rust のビルド結果に影響する。
  - GitHub Actions のキャッシュの上限はリポジトリあたり 10GB。

# Outcome

- 対象者は Releash の開発者、および PR の CI の完了を待つ workflow である。
- 現在、PR を出してから結果が返るまで約 24 分かかる。待ち時間を決めているのは Rust の検証を直列に並べた 1 ジョブで、その時間の大半は 24.5 万行の単一クレートを feature と profile の組み合わせを変えて繰り返しコンパイルする時間である。同じ構成のビルドが別ジョブと重複し、同じテストが 2 回走り、計測用の仕組みの自己確認がすべての PR で走る。Actions のキャッシュは上限を超えており、main のキャッシュが追い出されると依存クレートの再コンパイルが上乗せされる。coverage は main への push のたびに失敗し、main の CI が赤く見える状態が続いている。
- 変更後は、PR 層が merge 可否の判断に要る検証だけを並列・重複なしで実行して待ち時間を縮め、重い検証・構成の網羅・計測系は nightly 層が main の HEAD に対して日次と手動で実行する。coverage は失敗せずに計測できる。

# Current Behavior

最初の周の開始時点（`feat/issues/1850`、`4267f22f`）の状態。`.github/workflows/ci.yml`、`src-tauri/Cargo.toml`、`src-tauri/tests/desktop_cli_install.rs`、`package.json` を読み、Actions のキャッシュと ruleset を GitHub API で取得して確認した。run ごとの所要時間と coverage の失敗回数は Issue #1850 の実測記録による。

## workflow の構成

- `.github/workflows/` にあるのは `auto-tag.yml`、`bump-version.yml`、`ci.yml`、`codeql.yml`、`release.yml` の 5 本。`nightly.yml` は存在しない。
- `ci.yml` は `push`（main）と `pull_request`（main 宛）で起動し、`concurrency` を持たない。パスによる条件分岐も持たない。ジョブは `frontend` / `integration` / `rust-lint` / `rust-test` / `rust`（`rust-lint` と `rust-test` の集約）/ `quality` / `coverage`。
- `coverage` は `github.event_name == 'push' && github.ref == 'refs/heads/main'` のときだけ実行し、`pnpm exec vitest run --coverage` と `cargo llvm-cov --locked --codecov` の結果を Codecov へ送る。
- `rust-lint` のステップ: `cargo fmt --check` / `cargo clippy --locked -- -D warnings` / `cargo deny --locked check` / `cargo build --locked` / `cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings` / `cargo build --locked --no-default-features --bin releash-backend`。
- `rust-test` のステップ: `cargo test --locked --test agent_tui_harness` / `cargo test --locked` / `cargo test --locked --no-default-features --lib` / `cargo test --locked --no-default-features --features performance --lib cli::common::tests::` / `pnpm test:performance:daemon` / `cargo test --locked --no-default-features --test daemon_smoke`。
- `Swatinem/rust-cache@v2` のキーは `rust-lint` / `rust-test` / `rust-coverage`。`save-if` の指定は無く、PR でも保存する。
- `src-tauri/Cargo.toml` に `[profile]` 節が無く、dev / test は full debuginfo でビルドする。

## 待ち時間

| run | 日付 | 全体 | rust-test | rust-lint | frontend | integration | quality |
|---|---|---|---|---|---|---|---|
| #1201 PR | 2026-09-15 | 5.7 分 | 336s | 198s | 96s | 93s | 37s |
| #1202 PR | 2026-09-16 | 15.7 分 | 933s | 348s | 81s | 89s | 22s |
| #1829 PR（run 35423130048） | 2026-09-19 | 24.0 分 | 1434s | 636s | 105s | 107s | 40s |

`rust-test`（run 35423130048）の内訳。

| ステップ | 所要 | うちコンパイル | テスト実行 |
|---|---|---|---|
| apt・キャッシュの復元と保存 | 約 180s | — | — |
| Agent TUI harness self-test | 203s | 3m22s | 0.13s（13 件） |
| `cargo test --locked` | 352s | 4m01s | 約 110s |
| Headless backend unit tests | 187s | 2m45s | 21s（2741 件） |
| Performance CLI data directory tests | 107s | 1m46s | 0.00s（10 件） |
| Performance daemon helper self-test | 245s | 4m02s（release ビルド） | 数秒 |
| Headless daemon protocol smoke test | 145s | 2m15s | 10.7s |

- 依存クレートがキャッシュから復元できた run（35523093246）でも、`releash` 本体のコンパイルに各ステップ 1m50s〜4m01s かかる。
- `rust-lint` の `cargo build --locked`（198s）と `cargo build --locked --no-default-features --bin releash-backend`（136s）は、`rust-test` が別の runner でビルドするのと同じ構成である。統合テスト 8 本以上が `CARGO_BIN_EXE_releash-backend` を使うため、`cargo test` も同じ bin をビルドする。
- `agent_tui_harness` の 13 件は、直後の `cargo test --locked` でもう一度走る。

## 検証の網羅

- Performance CLI のステップは `cli::common::tests::` で 10 件に絞っており、`performance` feature で gate されているのはそのうち 1 件（`src-tauri/src/cli/common.rs:471`）。`performance` で分岐する他のテスト（`infrastructure/platform/app_data_dir_test.rs:9`、`client_api_acceptance.rs:441`、`infrastructure/platform/cli_install.rs:309`）は、`performance` を有効にした状態で一度も走っていない。
- `src-tauri/tests/desktop_cli_install.rs` は `#![cfg(all(debug_assertions, feature = "desktop", feature = "performance"))]` で gate されている。`desktop` と `performance` を同時に有効にするステップが `ci.yml` に無く、CI で一度も走っていない。

## キャッシュ

2026-09-21 時点で active なキャッシュは 9 件、合計 11,103,314,133 バイト（約 11.10GB）で上限の 10GB を超えている。内訳は `rust-test` 約 2,365MB × 2、`rust-lint` 約 2,409MB × 2、`rust-coverage` 約 1,011MB、CodeQL 約 61MB × 2、pnpm 約 140MB、Playwright 約 282MB。キャッシュの保存（post ステップ）に毎回 37〜117s かかる。

## coverage と必須チェック

- coverage は main への push で全テストをもう一度実行する（652s）。直近 7 回のうち 5 回が `llvm-profdata: no profile can be merged` で失敗している（run 35424251704、34939608186、34739610559、34712304439、34680458888）。テスト自体はすべて通っている。原因は未確認。
- ruleset `main` の必須チェックは `frontend` / `quality` / `integration` の 3 つで、`rust` は含まれない（`strict_required_status_checks_policy` は false）。Rust のジョブが失敗していても merge できる。

# Scope / Non-goals

Scope は次のとおり。

- `.github/workflows/ci.yml`: ジョブの分割、検証内容の重複の除去、`performance` 系と coverage の移出、キャッシュの保存条件とキー、debuginfo、`concurrency`、変更パスによる Rust の検証のスキップ。
- `.github/workflows/nightly.yml` の新設: 起動、`schedule` 起動時のスキップ判定、移した検証と追加した検証の実行。
- coverage の `llvm-profdata: no profile can be merged` の解消。
- `AGENTS.md` の「ビルド・テスト・Lint」の更新。

Non-goals は次のとおり。

- nightly 層からのリリース（prerelease の作成、stable の作成、古い nightly の削除、版番号の規則、署名・公証、appcast）。#1851 と #1766 が扱う。
- アプリが nightly を受け取る経路。
- `releash` クレートの分割。
- `frontend` / `integration` / `quality` の各ジョブの内容の変更。Playwright の `workers: 1` を含む。ただし `integration` が統合テストの結果を PR へ投稿するコメントの投稿条件は、R-017 の対象としてこの Non-goal の例外とする。判定式、コメントの本文、既存コメントの検索と更新の経路は変更しない。
- headless 構成（`--no-default-features`）の lib テストの nightly 層への移動。PR 層に残す。
- nightly 層で PR 層の検証を再実行すること。
- coverage を nightly 層のリリース可否の関門に含めるかの判断。#1851 が扱う。
- main へ merge した後の GitHub 上の作業。古いキャッシュの削除、PR の待ち時間・キャッシュ総量・apt install 所要の実測、apt パッケージのキャッシュを入れるかの判断、ruleset `main` の必須チェックへの `rust` の追加を含む。この変更が対象にするのはリポジトリのファイルだけである。
- PR 層の待ち時間の具体値の達成と検証。

# Requirements

- R-001: `ci.yml` は、PR と main への push の両方で同じジョブ一式を実行する。
- R-002: PR 層が実行する Rust の検証は、`cargo fmt --check`、既定 feature の `cargo clippy --locked -- -D warnings`、`cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings`、`cargo deny --locked check`、`cargo test --locked`、`cargo test --locked --no-default-features --lib`、`cargo test --locked --no-default-features --test daemon_smoke` である。同じ feature と profile の組み合わせのビルドまたはテストを 2 回以上実行しない。
- R-003: PR 層の Rust の検証は並列に実行し、一部が失敗しても他の検証の結果が得られる。
- R-004: PR 層は Rust の検証をまとめた `rust` という名前のチェックを持つ。Rust の検証がすべて成功したときだけ成功し、Rust の検証を実行しなかったときも成功する。
- R-005: PR 層は `performance` feature を使う検証と coverage の計測を実行しない。
- R-006: PR 層の Rust のビルドは debuginfo を生成しない。
- R-007: PR 層は main への push でだけ Rust のビルドキャッシュを保存し、PR では保存しない。キャッシュのキーはジョブごとに分ける。
- R-008: 同じ PR へ続けて push したとき、その PR の実行中の古い run は中断される。main への push で起動した run は中断されない。
- R-009: 変更が `docs/**` とリポジトリルート直下の `*.md` だけのとき、PR 層は Rust の検証を実行しない。この条件は PR と main への push の両方に適用する。`workflows/facets/**/*.md` を含む変更では実行する。
- R-011: `nightly.yml` は日次の `schedule` と `workflow_dispatch` で起動し、どちらで起動した場合も main の HEAD を対象に検証する。
- R-012: `schedule` で起動したとき、前回成功した nightly の run が起動した commit と main の HEAD が同じなら検証を実行しない。`workflow_dispatch` で起動したときは常に検証を実行する。
- R-013: nightly 層は `cargo test --locked --no-default-features --features performance --lib` を絞り込みなしで実行し、`pnpm test:performance:daemon` を release ビルドのまま実行する。
- R-014: nightly 層は `cargo test --locked --features performance --test desktop_cli_install` を実行する。
- R-015: nightly 層は TypeScript と Rust の coverage を計測し、`llvm-profdata: no profile can be merged` で失敗せずに結果を Codecov へ送る。
- R-016: `AGENTS.md` の「ビルド・テスト・Lint」に載るコマンドは、変更後の CI が実行するコマンドと一致する。
- R-017: PR 層が統合テストの結果を PR へ投稿するコメントは、run がキャンセルされたときには投稿も更新もされない。統合テストが失敗したときは失敗として報告する。

# Assumptions / Open Questions

- 自動判断: R-013 の lib テストの feature 構成を `--no-default-features --features performance` とした。原案の R-013 は「`performance` feature を有効にした lib テスト」とだけ書き、headless 構成か既定（desktop）構成かを定めていなかった。移出元のステップが `cargo test --locked --no-default-features --features performance --lib cli::common::tests::` であり、`client_api_acceptance.rs:441` の `performance` 分岐は `lib.rs:5` の `#[cfg(all(debug_assertions, feature = "desktop"))]` で desktop 構成にしか存在せず R-014 の `desktop_cli_install` が覆うため、既存の構成を維持し絞り込みだけを外す解釈を採った。
