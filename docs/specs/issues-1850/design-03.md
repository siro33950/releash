# Design 03

## 開始状態

差分の基準は `main`（派生点 `4267f22f`、ブランチ `feat/issues/1850`）。直前の Design は `docs/specs/issues-1850/design-02.md` で、その「変える部分」4 件は未コミットの変更として実装済みである（`ci.yml:69` の PR コメントステップの `if` を `!cancelled() && github.event_name == 'pull_request'` へ変更、`Workflow decision tests` を独立ジョブ `workflow-tests`（`ci.yml:109-118`）へ分離、`nightly.yml` の Codecov 送信 2 本への `fail_ci_if_error: true` の付与、`rust-test-headless` からの frontend 準備ステップの除去）。

この周までに Thread `7a3b0526` / `b2fc699a` / `75a520a6` / `aafa4928` が resolve されている。`[DEFERRED]` として人間へ渡した件は無い。

open Thread は `cd7d1e6a` と `c29c4b65` の 2 件で、いずれも `[FIX_POLICY]` が付いている。

## 変える部分

- 準備未完了の run で検証ステップを起動しない: `ci.yml` の `rust-lint`（`:179-187`）/ `rust-test-desktop`（`:226-227`）/ `rust-test-headless`（`:253-258`）の検証ステップの実行条件を、同じジョブの準備ステップが失敗した run では検証を起動しないものにする。準備が成功した run では、先行の検証の失敗で同じジョブの後続の検証を止めない挙動を保つ。`.github/scripts/workflows-test.mjs:136` がこの条件式を現在の文字列で固定しているため、期待値も合わせる。根拠: Thread `c29c4b65`。準備ステップ（`:149-178` / `:200-225` / `:240-252`）の `if` は `steps.changes.outputs.rust == 'true'` だけで既定の `success()` が付き、検証ステップは `!cancelled() && steps.changes.outputs.rust == 'true'` のため、apt 等の導入が失敗した run でも cargo の検証が起動する。維持する要求は R-003 / B-003（維持対象は検証どうしの部分失敗であり、準備未完了での起動は要求していない）、R-009 / B-011、R-004 / B-004。ルート: 委任。
- 禁止語 assertion の検査対象を実行されるステップに限る: `.github/scripts/workflows-test.mjs:76` の `assert.doesNotMatch(ciConfig, /performance|llvm-cov|coverage|cargo build|agent_tui_harness/)` が `ci.yml` を全文の文字列として検査しており、実行内容を変えない説明コメントでも失敗する。検査対象を、実行されるステップの内容だけに向ける。根拠: Thread `cd7d1e6a`。維持する要求は R-005 / B-005（定めるのは検証が実行されるかどうかであり、語の出現ではない）と R-016 / B-020。ルート: 委任。

## 固定するルート

今周に新たに固定する実装上の指定は無い。過去の周で固定し今周も維持するルートは次の 2 つで、いずれも上記 2 件の修正の範囲を外側から縛る。

1. design-02 の固定ルート 2 を維持する。`workflow-tests` は node だけで動く独立ジョブとして差分判定によらず常に実行し、集約ジョブ `rust` の `needs` には入れない。関係: R-004 / R-009 / R-016、B-004 / B-011 / B-020。
2. design-02 の固定ルート 4（design-01 の固定ルート 1 の一部）を維持する。`frontend` / `integration` / `quality` / `rust-lint` / `rust-test-desktop` / `rust-test-headless` / `rust` というジョブ名と、各 Rust ジョブが持つ検証の割り当てを変えない。集約チェックの名前 `rust` も変えない。関係: R-002 / R-004、B-002 / B-004。

## 変えないもの

- `rust-lint` / `rust-test-desktop` / `rust-test-headless` の準備ステップが導入する内容（apt のパッケージ、pnpm / Node、frontend 資産、toolchain、rust-cache、cargo-deny）。理由: Thread `c29c4b65` の対象は検証ステップの実行条件であり、準備の内容ではない。
- `workflows-test.mjs` の R-002 / R-004 に基づく既存 assertion（`ci.yml` のジョブ一覧と、各 Rust ジョブのコマンド列の照合）。理由: Thread `cd7d1e6a` の対象は禁止語 assertion の検査範囲に限る。

## 未確定・リスク

- 自動判断（design-01 から継続）: R-013 の lib テストの feature 構成を `--no-default-features --features performance` とした（`requirements.md` の Assumptions に記載）。原案の R-013 は headless 構成か既定（desktop）構成かを定めていなかった。既定構成と読むと headless 構成の `performance` 分岐が nightly 層でも未実行のまま残る。
- 今周で新たに自動判断した箇所は無い。「自動判断: 未決」として残した要求も無い。`[DEFERRED]` で人間へ渡した件も無い。
