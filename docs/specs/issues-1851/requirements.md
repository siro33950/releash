# Context

- 正本: Issue #1851「[02] nightly を基点にリリースする」。マイルストーン 96「00. Nightly 化と CI 改善」。
- 依存: #1850（CLOSED）。`.github/workflows/nightly.yml` を新設し、`check` / `performance` / `coverage` の 3 ジョブを置いた。nightly 層は常に main を対象にし、起動は日次 `schedule`（`23 18 * * *`）と `workflow_dispatch`。
- 分離元: #1766「B14: Xcode 署名・公証・Sparkle による公開」。Swift アプリのビルド（`xcodebuild`）と Sparkle による nightly / stable の配布は #1766 が持ち、本変更で作る起動・スキップ判定・関門・Release の流れの上に載せる。#1766 は B13 と #1851 に依存する。
- 対象ファイル: `.github/workflows/nightly.yml`、`.github/workflows/bump-version.yml`、`.github/workflows/auto-tag.yml`、`.github/workflows/release.yml`、stable 用の workflow（新設）、`AGENTS.md`。
- 現行アプリは Tauri。macOS の universal ビルドは `tauri-apps/tauri-action@v1`（`--target universal-apple-darwin -- --features vendored-openssl`）で作る。
- Apple 署名・公証の鍵、Tauri updater の署名鍵、telemetry の送信先は `1password/load-secrets-action` で build 時 env として注入する。注入している workflow は現在 `release.yml` だけで、対象は `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` / `APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` / `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` / `OTLP_ENDPOINT` / `NEW_RELIC_LICENSE_KEY`。OTLP の値を build 時 env で供給する方式は `docs/specs/issues-1209/design.md` の D7 が根拠。
- 現行アプリの更新は Tauri updater。取得先は `src-tauri/tauri.conf.json` の `plugins.updater.endpoints` の 1 本 `https://github.com/siro33950/releash/releases/latest/download/latest.json`。GitHub の `latest` は prerelease と draft を除く。
- 版は `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の 4 箇所に書く。
- Tauri は設定の `version` をそのまま macOS バンドルの `CFBundleShortVersionString` と、`bundle > macOS > bundleVersion` 未指定時の `CFBundleVersion` に書き込む。`CFBundleShortVersionString` はプレリリース識別子を想定しない形式である。
- PR 層の検証は `.github/workflows/ci.yml`。ジョブは `frontend` / `integration` / `workflow-tests` / `rust-lint` / `rust-test-desktop` / `rust-test-headless` / `rust`（集約）/ `quality`。main の ruleset（id 12693270）の必須チェックは `frontend` / `quality` / `integration` の 3 つで、`rust` は含まない。

# Outcome

対象者は、Releash のリリースを行う開発者と、Tauri updater で更新を受ける既存利用者。

開発者は、重い検証を通したうえでリリースする経路を持たない。`Release` はタグ push で起動し、テストの結果を待たずに macOS ビルドを作る。リリースのたびに版を上げる PR の作成と merge を挟む必要がある。

変更後は、main の commit に対して関門の検証が緑になったときだけ、署名・公証済みのビルドが prerelease の GitHub Release として上がる。stable はその nightly を指定して手動で作り、指定した commit からビルド・署名・公証をやり直す。既存利用者が Tauri updater で stable へ更新できる経路は変わらない。

# Current Behavior

リリースの流れ（3 本の workflow）。

- `Bump Version`（`.github/workflows/bump-version.yml`）は `workflow_dispatch` の `bump` 入力（`patch` / `minor` / `major`）で起動する。`src-tauri/tauri.conf.json` の版から次の版を計算し、上記 4 箇所を書き換えて、`peter-evans/create-pull-request@v8` で `release/vX.Y.Z` ブランチの PR（title `release: vX.Y.Z`）を作る。
- `Auto Tag`（`auto-tag.yml`）は main への push で `package.json` の版が直近のタグと異なるときに `vX.Y.Z` タグを作って push する。
- `Release`（`release.yml`）は `push: tags: ['v*']` で起動する。`create-release` ジョブが `draft: true` で Release を作り、`build` ジョブ（`macos-latest`）が 1Password から鍵と telemetry の値を読み、`tauri-action` で universal ビルドをその Release へ載せる。ジョブの `needs` は `create-release` だけで、テストの結果を待たない。

観測した結果。

- 調査開始時点の版は `0.4.15`。調査中に `release: v0.4.16` (#1858) が main へ merge され、`Auto Tag` が `v0.4.16` タグを作り、`Release` が asset 4 件を持つ draft の Release を作った。上記 3 本の流れは現在も動いている。
- `v0.4.15` の Release は `draft: false`、`prerelease: false`。asset は `latest.json`、`Releash_0.4.15_universal.app.tar.gz`（74.1MB）、`Releash_0.4.15_universal.app.tar.gz.sig`、`Releash_0.4.15_universal.dmg`（73.1MB）。
- `v0.3.5`〜`v0.4.16` の 100 タグに prerelease は無い。上げ幅の実績は patch 98 回、minor 1 回（`v0.4.0`）、major 0 回。stable の間隔は最大 6 日。
- 版を上げる PR の CI は、run 内でジョブが 1 つも走らずに失敗する。`release/v0.4.13` は 12 秒、`release/v0.4.14` は 14 秒、`release/v0.4.15` は 147 秒、`release/v0.4.16` は 268 秒で `failure`。いずれも run のジョブ数が 0 で、`CodeQL` も同じ結果になる。`actor` と `triggering_actor` は `github-actions[bot]`。必須チェックが報告されないまま merge されている。
- `nightly.yml` の `check` ジョブは、`workflow_dispatch` なら常に実行と判定し、`schedule` なら main で最後に成功した `nightly.yml` の run の `head_sha` と `context.sha` を比べる。nightly のタグは見ていない。
- `nightly.yml` の `permissions` は `contents: read` と `actions: read`。3 ジョブとも `ubuntu-latest` で、macOS のビルドジョブは無い。GitHub Release を作る手順も、古い Release を消す手順も無い。
- `nightly.yml` の唯一の実行（run 35556056820、`workflow_dispatch`）は `check` 5 秒、`coverage` 12 分 59 秒、`performance` 16 分 40 秒で、3 ジョブとも成功。`coverage` と `performance` は並列。
- `coverage` ジョブは `pnpm exec vitest run --coverage`、`python3 .github/scripts/coverage.test.py`、`cargo llvm-cov` を実行し、Codecov へのアップロードを `fail_ci_if_error: true` で 2 回行う。`coverage.test.py` は `ci.yml` にも `performance` ジョブにも無い。
- `AGENTS.md` の「リリース」節は、`Bump Version` → `Auto Tag` → `Release` の 3 手順を記述している。「コミット・PR」節は「リリースコミットは `release: vX.Y.Z`」と記述している。

# Scope / Non-goals

変更する。

- `.github/workflows/nightly.yml`: 関門の検証が緑のときに署名・公証済みのビルドを作り、prerelease の GitHub Release を作る経路と、古い nightly の Release を消す経路を足す。`schedule` 起動のスキップ判定を nightly のタグ基準に変える。関門に PR 層と同じ検証一式を足す。
- stable の Release を `workflow_dispatch` で作る経路と、その後に版を上げる PR を作る経路を新設する。
- `.github/workflows/auto-tag.yml`、`.github/workflows/release.yml`: 削除する。
- `AGENTS.md` の「リリース」節と「コミット・PR」節。

変更しない。

- アプリが nightly を受け取る経路は作らない。nightly は GitHub Release の prerelease から手で取得する。受け取る経路は #1766 の Sparkle の channel で作る。
- 署名と公証の鍵の取得元は 1Password のまま。
- `src-tauri/tauri.conf.json` の `plugins.updater.endpoints`。
- Swift アプリ向けのビルド手順（`xcodebuild`）、Sparkle の appcast、`MARKETING_VERSION` / `CURRENT_PROJECT_VERSION` の採番。#1766（B13 依存）が扱う。本変更で作る起動・スキップ判定・関門・Release の流れを #1766 がそのまま使う。
- `ci.yml` の PR 層の構成（#1850 で確定した内容）。
- `coverage` ジョブの内容と、その `no profile can be merged`（#1850 の対象）。
- `.github/workflows/bump-version.yml`。`Bump Version` は版を変える仕組みとして残る。R-009 が求める振る舞いは現状のこのファイルで満たされている。
- 版を上げる PR の CI がジョブを 1 つも実行せずに失敗する事象。原因が未確認であり、ファイル変更で解消できるかも未確定のため、別 Issue で扱う。
- main の ruleset の必須チェックの構成。

# Requirements

- R-001: nightly の関門は、`nightly.yml` の `performance` ジョブと、PR 層（`ci.yml`）と同じ検証一式である。関門がすべて成功したとき、その対象 commit に対して、署名・公証済みの macOS universal ビルドを asset に持つ prerelease の GitHub Release が作られる。関門が 1 つでも失敗したときは Release が作られない。`coverage` ジョブは関門に含まれず、その結果は Release を作るかどうかに影響しない。
- R-002: `schedule` で起動した nightly は、直近の nightly の Release のタグが指す commit と main の HEAD が同じとき、検証・ビルド・Release 作成のいずれも行わない。異なるときは main の HEAD を対象に実行する。
- R-003: `workflow_dispatch` で起動した nightly は、直近の nightly のタグと main の HEAD の関係によらず、main の HEAD を対象に検証・ビルド・Release 作成を行う。
- R-004: nightly の Release は直近 14 件だけが残り、それより古い nightly の Release は残らない。
- R-005: stable の Release は `workflow_dispatch` で、nightly のタグを入力として起動する。入力されたタグが指す commit からビルド・署名・公証をやり直した asset を持ち、タグ名は `v` + その commit のリポジトリに書かれた版とする。
- R-006: stable の Release は prerelease ではなく、`latest.json` を asset に持ち、GitHub の `latest` が指す状態で公開される。既存利用者は `plugins.updater.endpoints` を変えずに Tauri updater で stable へ更新できる。nightly の prerelease は Tauri updater の更新対象にならない。
- R-007: stable の Release が作られた後、`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の版を、patch を 1 つ上げた版へ揃える PR が作られる。
- R-008: nightly と stable のビルドには、Apple 署名・公証、Tauri updater の署名、telemetry の送信先が、現行 `Release` と同じ値で 1Password から注入される。
- R-009: `Auto Tag` と `Release` は無くなる。`v*` タグの push を起点にした Release 作成、および `package.json` の版変更を起点にしたタグ作成は行われない。`Bump Version` は版を変える仕組みとして残り、`workflow_dispatch` で指定した上げ幅（`patch` / `minor` / `major`）に従って上記 4 箇所の版を上げる PR を作る。
- R-010: `AGENTS.md` の「リリース」節が、nightly の起動とスキップ、stable の起動と入力、版を上げる PR、`Bump Version` による版の変更を含む変更後の流れを記述する。「コミット・PR」節の `release: vX.Y.Z` の記述が、リリースを表すものではなく版を上げるコミットを表すものとして改められる。
- R-011: nightly の Release のタグ名は `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}` とする。`X.Y.Z` は対象 commit のリポジトリに書かれた版、`YYYYMMDD` はビルドした日付、`{N}` はその日に作られた nightly の何本目かで 1 から始まり日ごとに戻る。リポジトリの 4 箇所に書く版は常に 3 つの整数からなり、プレリリース識別子を含まない。

# Assumptions / Open Questions

- 無し。
