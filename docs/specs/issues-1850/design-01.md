# Design 01

## 開始状態

初回。差分の基準は `main`（派生点 `4267f22f`、ブランチ `feat/issues/1850`）。未コミットの変更は `docs/specs/issues-1850/` の Requirements・Behavior だけで、`.github/workflows/` と `AGENTS.md` は `main` と同じである。実装の状態は `docs/specs/issues-1850/requirements.md` の Current Behavior を参照する。直前の Design は無い。この周までに解消・見送りとなった Thread は無い（open Thread なし）。

## 変える部分

- `rust-test` の分割: `ci.yml` の `rust-test` を `rust-test-desktop`（`cargo test --locked`）と `rust-test-headless`（`cargo test --locked --no-default-features --lib` と `cargo test --locked --no-default-features --test daemon_smoke`）の 2 ジョブに分け、集約ジョブ `rust` の `needs` を `rust-lint` / `rust-test-desktop` / `rust-test-headless` にする。根拠: R-003「PR 層の Rust の検証は並列に実行し、一部が失敗しても他の検証の結果が得られる」、B-003。ルート: 固定するルートの 1 番目。
- 重複ステップの削除: `rust-lint` から `cargo build --locked` と `cargo build --locked --no-default-features --bin releash-backend` を、`rust-test` から `Agent TUI harness self-test`（`cargo test --locked --test agent_tui_harness`）を削除する。根拠: R-002「同じ feature と profile の組み合わせのビルドまたはテストを 2 回以上実行しない」、B-002。ルート: 固定するルートの 2 番目。
- `performance` 系 2 ステップの移出: `Performance CLI data directory tests` と `Performance daemon helper self-test` を `ci.yml` から削除し、`nightly.yml` へ置く。前者は絞り込み `cli::common::tests::` を外して `cargo test --locked --no-default-features --features performance --lib` にする。根拠: R-005「PR 層は `performance` feature を使う検証と coverage の計測を実行しない」、R-013、B-005 / B-017。ルート: `pnpm test:performance:daemon` の release ビルド維持のみ固定するルートの 8 番目、他は委任。
- `coverage` ジョブの移出: `ci.yml` の `coverage` ジョブを削除し、`nightly.yml` へ置く。`ci.yml` から `github.event_name == 'push' && github.ref == 'refs/heads/main'` の条件付きジョブが無くなる。根拠: R-001「`ci.yml` は、PR と main への push の両方で同じジョブ一式を実行する」、R-005、R-015、B-001 / B-005 / B-019。ルート: 委任。
- coverage の失敗の解消: `llvm-profdata: no profile can be merged` で失敗せずに Rust の結果を Codecov へ送るようにする。根拠: R-015「`llvm-profdata: no profile can be merged` で失敗せずに結果を Codecov へ送る」、B-019。ルート: 委任（原因特定と修正方法）。
- rust-cache の保存条件とキー: `Swatinem/rust-cache@v2` に `save-if` を足して main のときだけ保存し、キーを `rust-lint` / `rust-test-desktop` / `rust-test-headless` のようにジョブごとに分ける。根拠: R-007「main への push でだけ Rust のビルドキャッシュを保存し、PR では保存しない。キャッシュのキーはジョブごとに分ける」、B-007 / B-008。ルート: 固定するルートの 3 番目。
- debuginfo の無効化: `ci.yml` の workflow レベルの `env` に `CARGO_PROFILE_DEV_DEBUG: "0"` を足す。根拠: R-006「PR 層の Rust のビルドは debuginfo を生成しない」、B-006。ルート: 固定するルートの 4 番目。
- `concurrency` の追加: 同じ PR の実行中の古い run を中断し、main への push で起動した run は中断しない。根拠: R-008、B-009 / B-010。ルート: 固定するルートの 5 番目。
- 文書だけの変更での Rust の検証のスキップ: 変更が `docs/**` とリポジトリルート直下の `*.md` だけのとき `rust-lint` / `rust-test-desktop` / `rust-test-headless` を実行しない。`workflows/facets/**/*.md` を含む変更では実行する。PR と main への push の両方に適用する。根拠: R-009、B-011 / B-012。ルート: 委任（判定の実現方法）。
- 集約ジョブ `rust` のスキップの扱い: Rust の検証を実行しなかったときも `rust` が成功する。根拠: R-004「Rust の検証がすべて成功したときだけ成功し、Rust の検証を実行しなかったときも成功する」、B-004。ルート: 委任（実現方法）。
- `nightly.yml` の新設: 日次の `schedule` と `workflow_dispatch` で起動し、checkout を `ref: main` に固定する。`schedule` 起動時は前回成功した nightly が対象にした commit と main の HEAD が同じなら以降を実行せず、`workflow_dispatch` では常に実行する。根拠: R-011、R-012、B-014 / B-015 / B-016。ルート: 固定するルートの 6 番目と 7 番目。ジョブの分け方と並列の単位は委任。
- `desktop_cli_install` の追加: nightly 層で `cargo test --locked --features performance --test desktop_cli_install` を実行する。`ci.yml` には無かった検証である。根拠: R-014、B-018。ルート: 委任。
- `AGENTS.md` の更新: 「ビルド・テスト・Lint」に載るコマンドを変更後の CI が実行するコマンドと一致させる。開始状態の `src-tauri/` の一覧は削除する `cargo build --locked` と `cargo test --locked --test agent_tui_harness` を含み、PR 層が実行する `cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings` / `cargo test --locked --no-default-features --lib` / `cargo test --locked --no-default-features --test daemon_smoke` を欠いている。根拠: R-016、B-020。ルート: 委任（文面）。

## 固定するルート

1. PR 層のジョブ構成と名前を固定する。`frontend` / `integration` / `quality`（いずれも内容は変更なし）、`rust-lint`（`cargo fmt --check` / `cargo clippy --locked -- -D warnings` / `cargo deny --locked check` / `cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings`）、`rust-test-desktop`（`cargo test --locked`）、`rust-test-headless`（`cargo test --locked --no-default-features --lib` と `cargo test --locked --no-default-features --test daemon_smoke`）、`rust`（集約。名前は変えない）。粒度: ジョブ名と各ジョブが持つ検証の割り当てまで。理由: 並列化の単位を Issue 本文が指定しており、`rust` は ruleset の必須チェックが参照する名前。関係: R-002 / R-003 / R-004、B-002 / B-003 / B-004。
2. `rust-lint` から `cargo build --locked` と `cargo build --locked --no-default-features --bin releash-backend` の 2 本、および `agent_tui_harness` 単独のステップを削除する。粒度: 削除するステップの指定まで。理由: 同じ構成のビルドを `rust-test` 側が行い、`agent_tui_harness` の 13 件は直後の `cargo test --locked` で再実行される。関係: R-002、B-002。
3. rust-cache は `Swatinem/rust-cache` の `save-if` で main のときだけ保存し、キーはジョブごとに分ける。粒度: 使うオプション名まで。理由: PR でのキャッシュ保存が上限超過と post ステップの 37〜117s を生んでいる。関係: R-007、B-007 / B-008。
4. CI の `env` で `CARGO_PROFILE_DEV_DEBUG: "0"` を設定して debuginfo を切る。粒度: 環境変数名と値まで。理由: `Cargo.toml` に `[profile]` 節を足さず CI 側だけで切る。関係: R-006、B-006。
5. `concurrency` で PR の古い run だけを止め、main への push では止めない。粒度: `concurrency` を使うことと、止める対象の指定まで。理由: Issue 本文の指定。関係: R-008、B-009 / B-010。
6. `nightly.yml` を新設し、起動は `schedule`（日次）と `workflow_dispatch` の 2 つ、checkout は `ref: main` に固定する。粒度: ファイル名、トリガーの種類、checkout の `ref` まで。理由: nightly 層は常に main を対象にする（マイルストーン #96）。関係: R-011、B-014。
7. `schedule` 起動時のスキップ判定は、前回成功した `nightly.yml` の run の `head_sha` を GitHub API から読み、`github.sha` と比較する。粒度: 参照する情報源（workflow run の `head_sha`）まで。比較の実装方法は委任。理由: tag や artifact のような追加の状態を持たない。関係: R-012、B-015 / B-016。
8. nightly 層へ移す `performance` 系のうち、`pnpm test:performance:daemon` は release ビルドのまま変えない。粒度: ビルド構成の維持まで。理由: Issue 本文の指定。関係: R-013、B-017。

## 変えないもの

- 集約チェックの名前 `rust`。範囲: `ci.yml` の集約ジョブ名。理由: ruleset `main` の必須チェックが参照する外部との契約。
- headless 構成（`--no-default-features`）の lib テストの実行場所。PR 層に残し nightly 層へ移さない。理由: Issue 本文の「変えないもの」。
- `frontend` / `integration` / `quality` の各ジョブの内容。Playwright の `workers: 1` を含む。理由: `integration` は約 100s で待ち時間に効いていない。
- `pnpm test:performance:daemon` の release ビルド。範囲: nightly 層へ移したあとも同じ。理由: Issue 本文の指定。
- nightly 層で PR 層の検証を再実行しないこと。理由: 検証を 2 層に分ける方針（マイルストーン #96）。

## 未確定・リスク

- 自動判断: R-013 の lib テストの feature 構成を `--no-default-features --features performance` とした（`requirements.md` の Assumptions に記載）。原案の R-013 は headless 構成か既定（desktop）構成かを定めていなかった。既定構成と読むと headless 構成の `performance` 分岐（`infrastructure/platform/app_data_dir_test.rs:9`、`infrastructure/platform/cli_install.rs:36` / `:67`）が nightly 層でも未実行のまま残る。
- coverage の `llvm-profdata: no profile can be merged` の原因が未特定である。R-015 / B-019 は失敗しないことを求めるが、原因が `cargo llvm-cov` の外（テストが起動する子プロセスの profile 出力など）にある場合、`nightly.yml` の構成だけでは解消できない可能性がある。
- 「自動判断: 未決」として残した要求は無い。`[DEFERRED]` で人間へ渡した件は無い。
