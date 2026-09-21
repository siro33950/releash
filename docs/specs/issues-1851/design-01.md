# Design 01

## 開始状態

初回。既存の `design-NN.md` は無く、開始状態は `requirements.md` の Current Behavior が記録した状態である。

- 差分の基準: base は `main`、派生点は `7948c6ec`。ブランチ `feat/issues/1851` にコード変更は無く、`docs/specs/issues-1851/` の Requirements・Behavior だけが未追跡で存在する。
- `main` は派生後に `release: v0.4.16` (#1858) を入れており、リポジトリ 4 箇所の版は `0.4.16`。派生点の版は `0.4.15`。
- 解消・見送りとなった Thread: 無し（初回のため Thread は存在しない）。

## 変える部分

- nightly のスキップ判定の基準: `schedule` 起動の判定を、`main` で最後に成功した `nightly.yml` の run の `head_sha` との比較から、直近の nightly の Release のタグが指す commit との比較へ変える。根拠: R-002「直近の nightly の Release のタグが指す commit と main の HEAD が同じとき、検証・ビルド・Release 作成のいずれも行わない」、B-003、B-004。ルート: D1。
- nightly の対象 commit: 関門の検証、ビルド、Release 作成が run 内で同一の対象 commit を指すようにする。根拠: B-001「その commit を指す `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}` 形式のタグを持つ prerelease の GitHub Release が作られる」。ルート: 委任。
- nightly の関門: `performance` に加えて PR 層（`ci.yml`）と同じ検証一式を関門にする。`coverage` は関門に含めない。根拠: R-001、B-001、B-002、B-013。ルート: D1。実行方法（`ci.yml` に `workflow_call` を足して呼ぶか、`nightly.yml` へ複製するか）は委任。
- nightly の macOS ビルド: 関門がすべて成功したときに、対象 commit から署名・公証済みの macOS universal ビルドを作るジョブを足す。Apple 署名・公証、Tauri updater の署名、telemetry の送信先を注入する。根拠: R-001、R-008、B-001。ルート: D2、D3、D4。
- nightly の Release 作成: `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}` のタグを持つ prerelease の GitHub Release を作り、ビルドを asset に載せる。根拠: R-001、R-011、B-001。ルート: D1、D2。`{N}` の採番方法は委任。
- 古い nightly の Release の削除: nightly の Release を直近 14 件だけ残し、それより古いものを消す。根拠: R-004、B-006。ルート: 委任。
- stable の Release 経路の新設: `workflow_dispatch` で nightly のタグを入力として起動し、そのタグが指す commit を checkout してビルド・署名・公証をやり直し、タグ名 `v` + その commit のリポジトリに書かれた版で、prerelease ではない Release を GitHub の `latest` が指す状態で公開する。`latest.json` を asset に持つ。根拠: R-005、R-006、R-008、B-007、B-008。ルート: D2、D3、D4。workflow の置き場所は委任。
- stable 後の版上げ PR: stable の Release が作られた後に、`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の版を patch を 1 つ上げた版へ揃える PR を作る。根拠: R-007、B-009。ルート: 委任。
- `.github/workflows/auto-tag.yml` の削除: `package.json` の版変更を起点にしたタグ作成を無くす。根拠: R-009、B-011。ルート: 委任。
- `.github/workflows/release.yml` の削除: `v*` タグの push を起点にした Release 作成を無くす。根拠: R-009、B-010。ルート: 委任。
- `AGENTS.md` の「リリース」節: nightly の起動とスキップ、stable の起動と入力、版を上げる PR、`Bump Version` による版の変更を含む変更後の流れへ書き換える。根拠: R-010、B-012。ルート: 委任。
- `AGENTS.md` の「コミット・PR」節: `release: vX.Y.Z` を、リリースではなく版を上げるコミットを表すものとして書き直す。根拠: R-010、B-012。ルート: 委任。

## 固定するルート

- D1: nightly の経路は `.github/workflows/nightly.yml` に足す。別ファイルを新設しない。範囲は nightly の起動・スキップ判定・関門・ビルド・Release 作成・古い Release の削除。粒度は配置先ファイルの指定のみで、ジョブ構成の指定は無い。
- D2: ビルドと Release への asset 添付は `tauri-action` を使う。範囲は nightly と stable の両方。粒度は道具の指定のみ。
- D3: 署名・公証の鍵は 1Password（`1password/load-secrets-action`）から取得する。範囲は nightly と stable の両方。粒度は取得元の指定のみ。
- D4: 起動・スキップ判定・関門・Release の流れを、B13 の後にビルドの手順だけを `xcodebuild` へ差し替えれば済む形に分ける。範囲は nightly と stable の構造。粒度は分離の境界（ビルド手順と、それ以外の流れ）の指定のみ。

## 変えないもの

- `src-tauri/tauri.conf.json` の `plugins.updater.endpoints`。既存利用者が Tauri updater で stable へ更新できる経路を維持するため。
- `ci.yml` の PR 層の構成。#1850 で確定した内容であるため。関門への流用でこの構成を変えない。
- `coverage` ジョブの内容。#1850 の対象であるため。関門から外すだけで、ジョブ自体の中身は変えない。
- main の ruleset の必須チェックの構成。

## 未確定・リスク

- 自動判断: `.github/workflows/bump-version.yml` を `requirements.md` の Scope の「変更する」から「変更しない」へ移した。Scope は「リリースを起こす前提だった記述を改める」としていたが、同ファイルにその記述は無く、R-009 と B-014 が求める振る舞いは現状で満たされているため。この判断により、`bump-version.yml` は今周の変える部分に含まれない。
