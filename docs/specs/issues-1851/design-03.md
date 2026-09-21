# Design 03

## 開始状態

- 直前の Design: `docs/specs/issues-1851/design-02.md`。
- 差分の基準: base は `main`、派生点は `a9a4cb6a`（`main` の HEAD と同一。design-02 と同じ）。ブランチ `feat/issues/1851` に commit は無く、design-01 の「変える部分」と design-02 の「変える部分」はすべて未コミットの作業ツリー変更として存在する（`.github/workflows/nightly.yml`、`.github/workflows/ci.yml`、`.github/scripts/workflows-test.mjs`、`AGENTS.md`、`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` の変更、`.github/workflows/auto-tag.yml` と `.github/workflows/release.yml` の削除、`.github/workflows/stable.yml` の新設）。
- design-02 の「変える部分」（リポジトリ 4 箇所の版を `0.4.17` へ上げる）は開始状態で満たされている。4 箇所とも `0.4.17` である。
- 解消した Thread: `111c98a8-6f6e-401f-946d-dd031b1ed297`（design-02 の周で `--outcome resolved`）。
- 今周の open Thread: `b47426a0-bb9f-4996-a0bc-16952d2180cc`、`e2e044bd-99c3-4e34-9cd7-fca0ac18ed6e`。いずれも `[FIX_POLICY]` が付いている。

## 変える部分

- nightly の関門の定義元 commit と対象 commit の一致: 関門として実行する検証一式の定義元 commit と、`create-release` 以降が対象とする commit を同一にする。一致しない場合は Release を作らない。R-002 のスキップ判定と R-003 の対象（main の HEAD）は維持する。根拠: Thread `b47426a0-bb9f-4996-a0bc-16952d2180cc`（`.github/workflows/nightly.yml:52` の相対 `uses` は呼び出し元 workflow と同じ commit の `ci.yml` 定義を読み込む一方、`check`（同 27-30 行）が実行時に `heads/main` を API で再解決した SHA を `with: ref`（同 53-54 行）へ渡すため、R-001 の「PR 層と同じ検証一式を関門にする」と R-003 の「main の HEAD を対象に検証・ビルド・Release 作成を行う」を、実行定義と対象コードが異なる状態では満たせない）。R-001、R-003、B-001、B-002。ルート: 委任。
- `workflows-test.mjs` のイベント名分岐の検証: `push` と `pull_request` のイベント値に対しても docs-only のスキップとコード変更時の実行を検証し、Rust 検査を強制実行するイベントの範囲を `push` や `pull_request` へ広げる変更でテストが失敗するようにする。`ci.yml` の docs-only スキップの条件自体は変更しない。根拠: Thread `e2e044bd-99c3-4e34-9cd7-fca0ac18ed6e`（`.github/workflows/ci.yml:148-151` の `rust-changes` は今周の差分で `context.eventName` を参照する分岐を追加したが、`.github/scripts/workflows-test.mjs:310-343` の `changes()` 呼び出しは `context` に `eventName` を与えないため、否定側が `undefined` でしか評価されない）。R-001。ルート: 委任。

## 固定するルート

- 今周に新しく固定するルートは無い（`materials.design.directions` は空）。
- design-01 の D1〜D4 と design-02 の D5〜D8 を今周も維持する（D1: nightly の経路は `.github/workflows/nightly.yml` に足す。D2: ビルドと Release への asset 添付は `tauri-action`。D3: 署名・公証の鍵は 1Password から取得。D4: 起動・スキップ判定・関門・Release の流れを、B13 の後にビルド手順だけを `xcodebuild` へ差し替えれば済む形に分ける。D5: 版上げの値は `0.4.17`、対象は 4 箇所のみ。D6: 初回移行のために `.github/workflows/stable.yml` へ既存タグの再利用・分岐を入れない。D7: 初回移行のために `requirements.md`・`behavior.md` へ移行専用の要求・受入条件を追加しない。D8: `.github/workflows/bump-version.yml` を変更しない）。解除の決定は無い。

## 変えないもの

- design-01 の「変えないもの」を今周も維持する。`src-tauri/tauri.conf.json` の `plugins.updater.endpoints`、`ci.yml` の PR 層の構成、`coverage` ジョブの内容、main の ruleset の必須チェックの構成。
- `ci.yml` の docs-only スキップの条件。`e2e044bd-99c3-4e34-9cd7-fca0ac18ed6e` の修正は検証の追加であり、判定の条件自体を変えない。

## 未確定・リスク

なし。
