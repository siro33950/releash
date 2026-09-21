# Design 02

## 開始状態

差分の基準は `main`（派生点 `4267f22f`、ブランチ `feat/issues/1850`）。直前の Design は `docs/specs/issues-1850/design-01.md` で、その「変える部分」は未コミットの変更として実装済みである（`.github/workflows/ci.yml` の修正、`.github/workflows/nightly.yml` と `.github/scripts/workflows-test.mjs` / `.github/scripts/coverage.test.py` の新設、`AGENTS.md` の「ビルド・テスト・Lint」の更新）。

この周までに Thread `1a49daf9`（nightly のスキップ判定が起動 ref の `head_sha` を使う）が resolve されている。実装は変えず R-012 / B-015 / B-016 の文言を実態へ合わせる改訂で決着した。`[DEFERRED]` として人間へ渡した件は無い。

open Thread は `7a3b0526` / `b2fc699a` / `75a520a6` / `aafa4928` の 4 件で、いずれも `[FIX_POLICY]` が付いている。

## 変える部分

- キャンセルされた run のテスト結果コメント: `ci.yml:69` の PR コメントステップの `if` 式を変え、キャンセルされた run がコメントを投稿も更新もしないようにする。根拠: R-017「PR 層が統合テストの結果を PR へ投稿するコメントは、run がキャンセルされたときには投稿も更新もされない。統合テストが失敗したときは失敗として報告する。」、B-021 / B-022、Thread `7a3b0526`。ルート: 固定するルートの 1 番目。
- workflow 判定テストの独立ジョブ化: `ci.yml:138-140` の `Workflow decision tests`（`node --test .github/scripts/workflows-test.mjs`）を `rust-lint` から出し、差分判定によらず常に実行する独立ジョブにする。文書だけの変更で Rust の検証を実行しないときに、このステップの失敗が集約チェック `rust` を失敗させる経路を無くす。根拠: R-004「Rust の検証を実行しなかったときも成功する」、R-009、R-016、B-004 / B-011 / B-020、Thread `b2fc699a`。ルート: 固定するルートの 2 番目。ジョブ名、`runs-on`、ステップ構成、および `workflows-test.mjs` 側の期待値（`ci.yml` のジョブ一覧の assertion と `rust-lint` のコマンド列の assertion）の更新は委任。
- Codecov への送信未完了を nightly の成功にしない: `nightly.yml:104-111` と `:125-132` の Codecov への送信が失敗した状態を、その run の成功として確定させない。TypeScript と Rust のいずれかの送信が失敗したとき、同じ main commit に対する次回の `schedule` 起動が検証をスキップしないようにする。根拠: R-015「nightly 層は TypeScript と Rust の coverage を計測し、`llvm-profdata: no profile can be merged` で失敗せずに結果を Codecov へ送る」、R-012、B-019 / B-015、Thread `75a520a6`。ルート: 委任。ただし固定するルートの 3 番目（スキップ判定の情報源）は変えない。
- `rust-test-headless` の準備ステップの整理: `ci.yml:237-249` の `pnpm/action-setup` / `actions/setup-node` / `pnpm install --frozen-lockfile` のうち、同ジョブの 2 検証（`cargo test --locked --no-default-features --lib` と `cargo test --locked --no-default-features --test daemon_smoke`）に必要でないものを除き、frontend 依存の取得を行わないようにする。根拠: Thread `aafa4928`。維持する要求は R-002 と B-002（検証の割り当ては変えない）。ルート: 委任。

## 固定するルート

1. Thread `7a3b0526` の修正: 変更するのは `ci.yml:69` の PR コメントステップの `if` 式だけで、`always() && github.event_name == 'pull_request'` を `!cancelled() && github.event_name == 'pull_request'` へ変える。粒度: 変更する式とその範囲まで。`ci.yml:75` の判定式 `passed`、`:76-84` のコメント本文の作り方、`:85-107` の既存コメントの検索と更新の経路は変えない。理由: キャンセルを「テスト失敗」として表示する案（判定式を `=== 'success'` へ変える）は実態と別の意味で不正確であり、Non-goals はこの投稿条件の一点だけを例外として緩めている。関係: R-017、B-021 / B-022。
2. Thread `b2fc699a` の修正: `ci.yml:138-140` の `Workflow decision tests` を `rust-lint` から出し、node だけで動く独立ジョブとして差分判定によらず常に実行する。集約ジョブ `rust` の `needs` は `rust-lint` / `rust-test-desktop` / `rust-test-headless` のままとし、この独立ジョブを入れない。粒度: ジョブを分けることと、`needs` へ入れないことまで。design-01 の固定ルート 1（PR 層のジョブ構成と名前）を、この独立ジョブ 1 本の追加で拡張する。既存ジョブの検証の割り当ては動かさない。理由: R-004 / R-009 / R-016 を同時に満たし、要求と Behavior の変更が要らない。`frontend` または `quality` へ足す案は Non-goals に触れ、`rust == 'true'` の条件下へ入れる案は `AGENTS.md` のみの変更で照合が一度も走らない穴を残し、`needs` へ入れると文書だけの変更で `rust` が failure になり R-004 に反する。関係: R-004 / R-009 / R-016、B-004 / B-011 / B-020。
3. design-01 の固定ルート 7 を維持する。nightly の `schedule` 起動時のスキップ判定の情報源は、前回成功した `nightly.yml` の run の `head_sha` である。tag / artifact / cache のような追加の状態を持たない。Thread `75a520a6` の修正はこの情報源を変えない範囲で行う。関係: R-012、B-015 / B-016。
4. design-01 の固定ルート 1 のうち、`frontend` / `integration` / `quality` / `rust-lint` / `rust-test-desktop` / `rust-test-headless` / `rust` というジョブ名と、各 Rust ジョブが持つ検証の割り当てを維持する。集約チェックの名前 `rust` も変えない。理由: ruleset `main` の必須チェックが参照する外部との契約。Thread `aafa4928` の修正は `rust-test-headless` のジョブ内の準備方法だけを対象とし、検証の割り当てには触れない。関係: R-002 / R-004、B-002 / B-004。

## 変えないもの

- `integration` のうち PR コメントの投稿条件以外の内容。Playwright の `workers: 1`、`playwright-cache`、artifact の upload を含む。`frontend` と `quality` も変えない。理由: Non-goals はこの投稿条件の一点だけを例外としている。
- `rust-lint` と `rust-test-desktop` の準備ステップ。理由: Thread `aafa4928` の対象は `rust-test-headless` のジョブ内の準備方法に限る。
- `rust-test-headless` の apt のパッケージ導入（`ci.yml:232-236`）。理由: `src-tauri/Cargo.toml` の Tauri の dev-dependency が無条件であるため、headless 構成のテストでも導入が要る。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-013 の lib テストの feature 構成を `--no-default-features --features performance` とした（`requirements.md` の Assumptions に記載）。原案の R-013 は headless 構成か既定（desktop）構成かを定めていなかった。既定構成と読むと headless 構成の `performance` 分岐が nightly 層でも未実行のまま残る。
- 今周で新たに自動判断した箇所は無い。「自動判断: 未決」として残した要求も無い。`[DEFERRED]` で人間へ渡した件も無い。
