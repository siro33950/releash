# Design 02

## 開始状態

- 直前の Design: `docs/specs/issues-1851/design-01.md`。
- 差分の基準: base は `main`、派生点は `a9a4cb6a`（`main` の HEAD と同一。design-01 の記録した派生点 `7948c6ec` から進んでいる）。ブランチ `feat/issues/1851` に commit は無く、design-01 の「変える部分」はすべて未コミットの作業ツリー変更として存在する（`.github/workflows/nightly.yml`、`.github/workflows/ci.yml`、`.github/scripts/workflows-test.mjs`、`AGENTS.md` の変更、`.github/workflows/auto-tag.yml` と `.github/workflows/release.yml` の削除、`.github/workflows/stable.yml` の新設）。
- リポジトリ 4 箇所（`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`）の版は `0.4.16`。`refs/tags/v0.4.16` は `a9a4cb6a` を指す。
- 解消した Thread: `111c98a8-6f6e-401f-946d-dd031b1ed297`（`--outcome resolved`）。open Thread は無い。

## 変える部分

- リポジトリの版を `0.4.17` へ上げる: `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の 4 箇所の版を `0.4.16` から `0.4.17` へ揃える。根拠: Thread `111c98a8-6f6e-401f-946d-dd031b1ed297`（`.github/workflows/stable.yml:58-66` は checkout した commit の `package.json` の版から `v` + 版のタグを組み立てて `createRef` するため、版が `0.4.16` のままでは既存タグ `v0.4.16` と衝突し、R-005 と B-007 の Release 作成へ到達しない）。ルート: D5。

## 固定するルート

- D5: 版上げの値は `0.4.17`（`main` の版 `0.4.16` の patch +1。design-01 が記録した派生点の版 `0.4.15` を基準にしない）。対象は `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の 4 箇所のみ。粒度は値と対象ファイルの指定だけで、4 ファイルの編集方法、`main` との差分の取り込み方、版上げを差分に含める順序の指定は無い。
- D6: 初回移行のために `.github/workflows/stable.yml` へ既存タグの再利用・分岐を入れない。範囲は stable の Release 作成経路。理由は、D5 で解消する一度きりの移行のために恒久的な挙動を増やさないこと。
- D7: 初回移行のために `requirements.md` へ移行専用の Requirement を、`behavior.md` へ移行専用の受入条件を追加しない。理由は、移行を merge されるファイル変更で完結させ、正本へ残す要求にしないこと。
- D8: `.github/workflows/bump-version.yml` を変更しない。R-009 と B-014 が求める振る舞いを現状のファイルが満たすことを人間が確認済み。
- design-01 の D1〜D4 を今周も維持する（D1: nightly の経路は `.github/workflows/nightly.yml` に足す。D2: ビルドと Release への asset 添付は `tauri-action`。D3: 署名・公証の鍵は 1Password から取得。D4: 起動・スキップ判定・関門・Release の流れを、B13 の後にビルド手順だけを `xcodebuild` へ差し替えれば済む形に分ける）。解除の決定は無い。

## 変えないもの

- design-01 の「変えないもの」を今周も維持する。`src-tauri/tauri.conf.json` の `plugins.updater.endpoints`、`ci.yml` の PR 層の構成、`coverage` ジョブの内容、main の ruleset の必須チェックの構成。

## 未確定・リスク

なし。
